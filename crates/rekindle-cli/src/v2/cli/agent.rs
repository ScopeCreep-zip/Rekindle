//! Agent identity management CLI types.

use clap::Subcommand;

/// Agent lifecycle subcommands.
#[derive(Subcommand)]
pub enum AgentCmd {
    /// Register an agent identity with the daemon.
    Register {
        /// Agent name (alphanumeric, hyphens, underscores, 1-64 chars).
        name: String,
        /// Agent type: human, ai-llm, bot, filter, analyzer, bridge, system.
        #[arg(long, default_value = "bot")]
        agent_type: String,
        /// Agent capabilities (comma-separated).
        #[arg(long, value_delimiter = ',')]
        capabilities: Vec<String>,
    },
    /// Revoke an agent's registration.
    Revoke {
        /// Agent name to revoke.
        name: String,
    },
}
