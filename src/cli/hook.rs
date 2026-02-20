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
    /// Deliver pending narration before a tool executes.
    PreToolUse {
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,
    },
    /// Deliver pending narration after a tool executes.
    PostToolUse {
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,
    },
}

impl HookEvent {
    /// Execute a hook event.
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            HookEvent::SessionStart { agent } => {
                let agent = resolve_agent(&agent)?;
                crate::hook::session_start(agent)
            }
            HookEvent::UserPrompt { agent } => {
                let agent = resolve_agent(&agent)?;
                crate::hook::user_prompt(agent, None)
            }
            HookEvent::Stop { agent } => {
                let agent = resolve_agent(&agent)?;
                crate::hook::check_narration(agent, crate::hook::HookType::Stop)
            }
            HookEvent::PreToolUse { agent } | HookEvent::PostToolUse { agent } => {
                let agent = resolve_agent(&agent)?;
                crate::hook::check_narration(agent, crate::hook::HookType::ToolUse)
            }
        }
    }
}

fn resolve_agent(name: &str) -> anyhow::Result<&'static dyn crate::agent::Agent> {
    crate::agent::backend_by_name(name).ok_or_else(|| anyhow::anyhow!("unknown agent: {name}"))
}
