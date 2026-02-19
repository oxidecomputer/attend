use clap::Subcommand;

/// Value parser that validates agent names against registered backends.
pub(super) fn agent_value_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(crate::agent::AGENTS.iter().map(|a| a.name()))
}

/// Value parser that validates editor names against registered backends.
pub(super) fn editor_value_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(crate::editor::EDITORS.iter().map(|e| e.name()))
}

/// Hook event subcommands with a mandatory --agent flag.
#[derive(Subcommand)]
pub enum HookEvent {
    /// Clear cache and emit instructions for a new session.
    SessionStart {
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,
    },
    /// Emit editor context for a user prompt.
    UserPrompt {
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,
    },
    /// Deliver pending narration when the session stops.
    Stop {
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,
    },
}

impl HookEvent {
    /// Execute a hook event.
    pub fn run(self) -> anyhow::Result<()> {
        let (agent_name, event) = match self {
            HookEvent::SessionStart { agent } => (agent, crate::agent::HookEvent::SessionStart),
            HookEvent::UserPrompt { agent } => (agent, crate::agent::HookEvent::UserPrompt),
            HookEvent::Stop { agent } => (agent, crate::agent::HookEvent::Stop),
        };
        let agent = crate::agent::backend_by_name(&agent_name)
            .ok_or_else(|| anyhow::anyhow!("unknown agent: {agent_name}"))?;
        agent.run_hook(event, None)
    }
}
