use anyhow::Result;

pub enum BotAction {
    Spawn { name: String },
    List,
    Stop { name: String },
}

pub fn execute(action: BotAction) -> Result<()> {
    match action {
        BotAction::Spawn { name } => {
            // TODO: avvia bot Mineflayer via subprocess Node.js
            println!("Spawning bot '{name}'... [not yet implemented]");
        }
        BotAction::List => {
            println!("Active bots: [not yet implemented]");
        }
        BotAction::Stop { name } => {
            println!("Stopping bot '{name}'... [not yet implemented]");
        }
    }
    Ok(())
}
