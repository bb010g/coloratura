use std::borrow::Cow;

use failure::Error;
use memchr::Memchr2;
use serenity::client::Context;
use serenity::framework::standard::{Args as SArgs, Command, CommandError};
use serenity::model::channel::Message;

pub struct CmdFn<F>(pub F);
impl<F: Sync + Send + 'static + Fn(&mut Context, &Message, Args) -> Result<(), Error>> Command
    for CmdFn<F>
{
    fn execute(&self, ctx: &mut Context, msg: &Message, args: SArgs) -> Result<(), CommandError> {
        match self.0(ctx, msg, Args::new(args.full())) {
            Ok(()) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Args<'a> {
    message: Option<&'a str>,
}
impl<'a> Args<'a> {
    pub fn new(message: &'a str) -> Args {
        Args {
            message: match message {
                msg if !msg.is_empty() => Some(msg),
                _ => None,
            },
        }
    }
}
impl<'a> Iterator for Args<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Cow<'a, str>> {
        let message = match self.message {
            Some(m) => m,
            None => return None,
        };
        if message.starts_with('"') {
            let mut out: Option<String> = None;
            let mut last_bs: Option<usize> = None;
            let mut from = 1;
            let mut to = 0;

            let msg_bytes = message.as_bytes();
            let mut memchr2 = Memchr2::new(b'"', b'\\', msg_bytes).skip(1);

            macro_rules! push_esc {
                    ($memchr2:expr, $bs:expr) => ({
                        last_bs = None;
                        $memchr2.next();
                        out.get_or_insert_with(String::new).push_str(&message[from..$bs]);
                        from = $bs;
                    })
                }
            while let Some(i) = memchr2.next() {
                match *unsafe { msg_bytes.get_unchecked(i) } {
                    b'\\' => match last_bs {
                        Some(bs) => push_esc!(memchr2, bs),
                        None => last_bs = Some(i),
                    },
                    b'"' => match last_bs {
                        Some(bs) => push_esc!(memchr2, bs),
                        None => {
                            to = i;
                            break;
                        }
                    },
                    _ => unreachable!(),
                }
            }
            self.message = if to == 0 {
                to = message.len();
                None
            } else {
                match message[to..].trim_left() {
                    new if !new.is_empty() => Some(new),
                    _ => None,
                }
            };

            Some(match out {
                Some(mut out) => {
                    out.push_str(&message[from..to]);
                    Cow::Owned(out)
                }
                None => Cow::Borrowed(&message[1..to]),
            })
        } else {
            match message.find(' ') {
                None => {
                    self.message = None;
                    Some(Cow::Borrowed(message))
                }
                Some(ws) => {
                    let (arg, new) = message.split_at(ws);
                    self.message = match new.trim_left() {
                        new if !new.is_empty() => Some(new),
                        _ => None,
                    };
                    Some(Cow::Borrowed(arg))
                }
            }
        }
    }
}
