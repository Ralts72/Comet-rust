mod documents;
mod export;
mod objects;
mod project;
mod scenes;
mod session;
mod ui;
fn main() -> anyhow::Result<()> {
    session::run()
}
