mod documents;
mod export;
mod project;
mod scenes;
mod session;
mod ui;
fn main() -> anyhow::Result<()> {
    session::run()
}
