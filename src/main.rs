extern crate dotenv;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
extern crate memchr;
extern crate regex;
extern crate serenity;
extern crate tinycdb;
extern crate typemap;

use std::env;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str;
use std::sync::Arc;

use dotenv::dotenv;
use failure::{Compat, Error};
use regex::Regex;
use serenity::builder::CreateMessage;
use serenity::client::{Client, Context, EventHandler};
use serenity::client::bridge::gateway::{ShardId, ShardManager};
use serenity::framework::standard::{help_commands, DispatchError, HelpBehaviour, StandardFramework};
use serenity::http;
use serenity::model::channel::{Channel, Message};
use serenity::model::gateway::{Game, Ready};
use serenity::model::id::{RoleId, UserId};
use serenity::model::permissions::Permissions;
use serenity::prelude::Mutex;
use serenity::utils::Colour as SColour;
use typemap::Key;

mod db;
mod util;

use util::{Args, CmdFn};

struct ShardManagerContainer;
impl Key for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

struct Handler {}

impl EventHandler for Handler {
    fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

// If main is so good, why haven't they made a main 2?
#[inline(always)]
fn main2() -> Result<(), Error> {
    let token = env::var("DISCORD_TOKEN").map_err(|_| format_err!("DISCORD_TOKEN not set"))?;

    let handler = Handler {};
    let mut client =
        Client::new(&token, handler).map_err(|e| format_err!("Error creating client: {}", e))?;

    {
        let mut data = client.data.lock();
        data.insert::<ShardManagerContainer>(Arc::clone(&client.shard_manager));
    }

    let owners = {
        let mut set = HashSet::new();
        http::get_current_application_info()
            .map(|info| set.insert(info.owner.id))
            .map_err(|e| format_err!("Couldn't get application info: {:?}", e))?;
        if let Ok(eo) = env::var("EXTRA_OWNERS") {
            eo.split(',')
                .map(|o| {
                    o.parse::<u64>()
                        .map(|uid| {
                            set.insert(UserId(uid));
                        })
                        .map_err(|e| format_err!("Not a valid UID: {}", e))
                })
                .collect::<Result<(), _>>()?;
        }
        set
    };

    client.with_framework(
        StandardFramework::new()
            .configure(|c| c.owners(owners).prefix("%"))
            .on_dispatch_error(|_ctx, msg, err| match err {
                DispatchError::RateLimited(seconds) => {
                    let _ = msg.reply(&format!("Try again in {} seconds.", seconds));
                }
                _ => {}
            })
            .after(|_ctx, msg, _cmd_name, res| match res {
                Ok(()) => {}
                Err(e) => {
                    let _ = msg.reply(&format!("Error: {}", e.0));
                }
            })
            .simple_bucket("color", 1)
            .cmd("about", CmdFn(about))
            .cmd("latency", CmdFn(latency))
            .cmd("ping", CmdFn(ping))
            .group("Owner", |g| {
                // Can't use cmd because it doesn't carry over group config
                g.owners_only(true)
                    .command("presence", |c| c.cmd(CmdFn(presence)))
                    .command("quit", |c| c.cmd(CmdFn(quit)))
            })
            .group("Color", |g| {
                // Can't use cmd because it doesn't carry over group config
                g.prefix("color")
                    .bucket("color")
                    .guild_only(true)
                    .command("set", |c| c.cmd(CmdFn(color_set)))
                    .command("unset", |c| c.cmd(CmdFn(color_unset)))
                    .command("clean", |c| {
                        c.cmd(CmdFn(color_clean))
                            .required_permissions(Permissions::ADMINISTRATOR)
                    })
            })
            .customised_help(help_commands::with_embeds, |c| {
                c.lacking_permissions(HelpBehaviour::Strike)
                    .lacking_role(HelpBehaviour::Strike)
                //.lacking_ownership(HelpBehaviour::Hide)
            }),
    );

    client
        .start_autosharded()
        .map_err(|e| format_err!("Client error: {}", e))
}

fn about_msg(m: CreateMessage) -> CreateMessage {
    m.embed(|e| {
        e.title("About").description(concat!(
            "Coloratura is a color-management bot by <@!72791153467990016>. ",
            "She's written in Rust with ",
            "[Serenity](https://github.com/zeyla/serenity).",
            "Source is [available](https://github.com/bb010g/coloratura) ",
            "under MIT/Apache-2.0 dual license.",
            "\n\n",
            "Add her to your server with [this link](",
            "https://discordapp.com/api/oauth2/authorize",
            "?client_id=414271219219824650&permissions=268700736&scope=bot",
            ")."
        ))
    })
}

// Utility commands

fn about(_: &mut Context, msg: &Message, _: Args) -> Result<(), Error> {
    let _ = match msg.channel() {
        Some(Channel::Group(ch)) => ch.read().send_message(about_msg),
        Some(Channel::Guild(ch)) => ch.read().send_message(about_msg),
        Some(Channel::Private(ch)) => ch.read().send_message(about_msg),
        _ => bail!("Your channel setup is strange"),
    };
    Ok(())
}

fn ping(_: &mut Context, msg: &Message, _: Args) -> Result<(), Error> {
    let _ = msg.reply("Pong!");
    Ok(())
}

fn latency(ctx: &mut Context, msg: &Message, _: Args) -> Result<(), Error> {
    let data = ctx.data.lock();

    let shard_manager = data.get::<ShardManagerContainer>()
        .ok_or_else(|| format_err!("There was a problem getting the shard manager"))?;
    let manager = shard_manager.lock();
    let runners = manager.runners.lock();

    let runner = runners
        .get(&ShardId(ctx.shard_id))
        .ok_or_else(|| format_err!("No shard found"))?;

    let _ = msg.reply(&format!(
        "The shard latency is {}",
        runner
            .latency
            .map(|l| format!("{}.{}s", l.as_secs(), l.subsec_nanos()))
            .unwrap_or(String::from("unknown"))
    ));
    Ok(())
}

// Owner commands

fn presence(ctx: &mut Context, msg: &Message, mut args: Args) -> Result<(), Error> {
    let game = match args.next() {
        Some(ref arg) if arg == "playing" => {
            let name = args.next().ok_or_else(|| format_err!("Name needed."))?;
            Some(Game::playing(&name))
        }
        Some(ref arg) if arg == "streaming" => {
            let name = args.next().ok_or_else(|| format_err!("Name needed."))?;
            let url = args.next().ok_or_else(|| format_err!("URL needed."))?;
            Some(Game::streaming(&name, &url))
        }
        Some(ref arg) if arg == "listening" => {
            let name = args.next().ok_or_else(|| format_err!("Name needed."))?;
            Some(Game::listening(&name))
        }
        Some(ref arg) if arg == "reset" => None,
        _ => {
            bail!(
                "Give `playing <name>`, `streaming <name> <url>`, `listening <name>`, or `none`."
            );
        }
    };
    match game {
        Some(game) => ctx.set_game(game),
        None => ctx.reset_presence(),
    };
    let _ = msg.reply("Done!");
    Ok(())
}

fn quit(ctx: &mut Context, msg: &Message, mut args: Args) -> Result<(), Error> {
    match args.next() {
        None => {
            let _ = msg.reply("Quitting all shards.");

            let data = ctx.data.lock();
            let shard_manager = data.get::<ShardManagerContainer>()
                .ok_or_else(|| format_err!("There was a problem getting the shard manager"))?;
            shard_manager.lock().shutdown_all();
            Ok(())
        }
        Some(ref arg) if arg == "shard" => {
            let _ = msg.reply("Quitting current shard.");

            ctx.quit();
            Ok(())
        }
        Some(arg) => {
            bail!("Unknown argument \"{}\".", arg);
        }
    }
}

// Color commands

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Color(u8, u8, u8);
impl str::FromStr for Color {
    type Err = Compat<Error>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref RE: Regex =
                Regex::new(r"^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$")
                    .expect("Color regex failed compilation");
        }
        let s = s.to_lowercase();
        let caps = RE.captures(&s)
            .ok_or_else(|| format_err!("\"{}\" is not a RGB hex color", s).compat())?;

        let re_unwrap = "Color regex captures should always be present";
        let r = caps.get(1).expect(re_unwrap);
        let g = caps.get(2).expect(re_unwrap);
        let b = caps.get(3).expect(re_unwrap);

        let r = u8::from_str_radix(r.as_str(), 16)
            .map_err(|e| format_err!("Red parsing error: {}", e).compat())?;
        let g = u8::from_str_radix(g.as_str(), 16)
            .map_err(|e| format_err!("Green parsing error: {}", e).compat())?;
        let b = u8::from_str_radix(b.as_str(), 16)
            .map_err(|e| format_err!("Blue parsing error: {}", e).compat())?;

        Ok(Color(r, g, b))
    }
}
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}
impl From<Color> for SColour {
    fn from(c: Color) -> Self {
        SColour::from_rgb(c.0, c.1, c.2)
    }
}

fn color_set(_: &mut Context, msg: &Message, mut args: Args) -> Result<(), Error> {
    let guild = msg.guild()
        .ok_or_else(|| format_err!("This command should only run in guilds."))?;

    let color = args.next()
        .ok_or_else(|| format_err!("You must provide a hex RGB color."))
        .and_then(|a| {
            a.parse::<Color>()
                .map_err(|e| format_err!("Color parsing: {}", e))
        })?;
    let color_str = format!("{}", color);
    let color_bytes = color_str.as_bytes();

    let author_id = msg.author.id;
    let author_id_str = format!("{}", author_id);
    let author_id_bytes = author_id_str.as_bytes();

    let guild_id = { guild.read().id };
    let guild_str = format!("{}", guild_id);
    let data = db::data(&guild_str);

    db::ensure_dir(&data)?;
    let mut users = db::Guild::Users.open(&data)?;

    let old_role = users.as_mut().and_then(|db| {
        db.find(author_id_bytes)
            .and_then(|b| str::from_utf8(b).ok())
            .and_then(|s| s.parse::<RoleId>().ok())
            .and_then(|id| guild.write().roles.get(&id).map(|r| r.id))
    });
    let mut colors = db::Guild::Colors.open(&data)?;
    let role = colors
        .as_mut()
        .and_then(|colors| {
            colors
                .find(color_str.as_bytes())
                .and_then(|b| str::from_utf8(b).ok())
                .and_then(|s| s.parse::<RoleId>().ok())
                .and_then(|id| guild.write().roles.get(&id).map(|r| r.id))
        })
        .map(Ok)
        .unwrap_or_else(|| {
            db::Guild::Colors.rm_tmp(&data)?;
            let colour = SColour::from(color);
            guild
                .write()
                .create_role(|r| {
                    r.name(&format!("coloratura#{}", color_str))
                        .permissions(Permissions::empty())
                        .colour(colour.0 as u64)
                })
                .map(|r| r.id)
                .map_err(|e| format_err!("Color role creation failed: {:?}", e))
                .and_then(|role| {
                    db::Guild::Colors.set(
                        &data,
                        |ndb| {
                            for (k, v) in colors.as_mut().iter_mut().flat_map(|i| i.iter()) {
                                if k != color_bytes {
                                    let _ = ndb.add(k, v);
                                }
                            }
                            let _ = ndb.add(color_bytes, &format!("{}", role).as_bytes());
                        },
                        |_| role,
                    )
                })
        })?;
    let role_str = format!("{}", role);
    let role_bytes = role_str.as_bytes();

    if let Some(old_role) = old_role {
        guild
            .write()
            .members
            .get_mut(&author_id)
            .ok_or_else(|| format_err!("User isn't in members?"))
            .and_then(|m| {
                m.remove_role(old_role).map_err(|e| {
                    format_err!(
                        "Couldn't remove user from old color role {}: {}",
                        old_role,
                        e
                    )
                })
            })?;
    };

    db::Guild::Users.rm_tmp(&data)?;

    guild
        .write()
        .members
        .get_mut(&author_id)
        .ok_or_else(|| format_err!("User isn't in members?"))
        .and_then(|m| {
            m.add_role(role)
                .map_err(|e| format_err!("Couldn't add user to new color role {}: {}", role, e))
        })?;

    db::Guild::Users.set(
        &data,
        |ndb| {
            for (k, v) in users.as_mut().iter_mut().flat_map(|i| i.iter()) {
                if k != author_id_bytes {
                    let _ = ndb.add(k, v);
                }
            }
            let _ = ndb.add(author_id_bytes, role_bytes);
        },
        |_| (),
    )?;

    let _ = msg.reply(&format!("Your color is now #{}.", color));

    Ok(())
}

fn color_unset(_: &mut Context, msg: &Message, _: Args) -> Result<(), Error> {
    let guild = msg.guild()
        .ok_or_else(|| format_err!("This command should only run in guilds."))?;

    let author_id = msg.author.id;
    let author_id_str = format!("{}", author_id);
    let author_id_bytes = author_id_str.as_bytes();

    let guild_id = { guild.read().id };
    let guild_str = format!("{}", guild_id);
    let data = db::data(&guild_str);

    let mut users = db::Guild::Users
        .open(&data)
        .and_then(|db| db.ok_or_else(|| format_err!("There are no colors for this guild.")))?;

    let role = users
        .as_mut()
        .find(author_id_bytes)
        .and_then(|b| str::from_utf8(b).ok())
        .and_then(|s| s.parse::<RoleId>().ok())
        .and_then(|id| guild.read().roles.get(&id).map(|r| r.id))
        .ok_or_else(|| format_err!("You have no active color."))?;

    db::Guild::Users.rm_tmp(&data)?;

    guild
        .write()
        .members
        .get_mut(&author_id)
        .ok_or_else(|| format_err!("User isn't in members?"))
        .and_then(|m| {
            m.remove_role(role).map_err(|e| {
                format_err!("Couldn't remove user from old color role {}: {}", role, e)
            })
        })?;

    db::Guild::Users.set(
        &data,
        |ndb| {
            for (k, v) in users.iter() {
                if k != author_id_bytes {
                    let _ = ndb.add(k, v);
                }
            }
        },
        |_| (),
    )?;

    let _ = msg.reply(&format!("Your color has been unset."));
    Ok(())
}

fn color_clean(_: &mut Context, msg: &Message, _: Args) -> Result<(), Error> {
    let guild = msg.guild()
        .ok_or_else(|| format_err!("This command should only run in guilds."))?;
    let guild_id = { guild.read().id };
    let guild_str = format!("{}", guild_id);
    let data = db::data(&guild_str);

    let mut colors = db::Guild::Colors.open(&data)?;
    let mut colors = colors.as_mut();
    let mut users = db::Guild::Users.open(&data)?;
    let mut users = users.as_mut();

    let mut roles_used: HashMap<&[u8], bool> = HashMap::new();

    let mut colors_cache = HashMap::new();
    for (color, role) in colors.iter_mut().flat_map(|db| db.iter()) {
        colors_cache.insert(color, role);
        roles_used.insert(role, false);
    }
    for (_user, role) in users.iter_mut().flat_map(|db| db.iter()) {
        roles_used.insert(role, true);
    }

    db::Guild::Colors.rm_tmp(&data)?;

    {
        // If you don't clone or get a fresh read lock every iteration,
        // you'll deadlock when starting your third deletion.
        // I have no idea why this happens.
        let roles = { &guild.read().roles.clone() };
        for (role_id_bytes, used) in &roles_used {
            if *used {
                continue;
            }
            let role_id_str = str::from_utf8(role_id_bytes)
                .map_err(|e| format_err!("Role ID `{:?}` wasn't UTF-8: {}", role_id_bytes, e))?;
            let role_id = role_id_str
                .parse::<RoleId>()
                .map_err(|e| format_err!("Role ID {} wasn't a valid RoleID: {}", role_id_str, e))?;
            if let Some(role) = roles.get(&role_id) {
                role.delete()
                    .map_err(|_| format_err!("Failed to delete {}.", role_id))?;
            }
        }
    }

    db::Guild::Colors.set(
        &data,
        |ndb| {
            for (color, role_id) in &colors_cache {
                if *roles_used.get(role_id).unwrap_or(&true) {
                    let _ = ndb.add(color, role_id);
                }
            }
        },
        |_| (),
    )?;

    let _ = msg.reply("Colors cleaned.");

    Ok(())
}

fn main() {
    dotenv().ok();

    match main2() {
        Ok(_) => {}
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    }
}
