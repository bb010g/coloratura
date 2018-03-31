# kamasutra

Coloratura is a color-management bot for Discord built with [Serenity].
It uses [tinycdb] for storage.

## Usage

Just `cargo run --release` with your `DISCORD_TOKEN` in the environment (or
`.env`) and go. Data will be stored under `./data/$guild_id/`.

[Serenity]: https://github.com/zeyla/serenity
[tinycdb]: https://github.com/andrew-d/tinycdb-rs
