//! Metadata for the services `sbx secret set` recognises.
//!
//! The Secrets table and its info dialog both explain where a stored secret is
//! used: the environment variable that reads it and the API domains that can
//! receive it. Keeping that list here means the two views cannot disagree.

/// The environment variables and API domains associated with one service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: &'static str,
    pub env_vars: &'static [&'static str],
    pub api_domains: &'static [&'static str],
}

const SERVICES_INFO: &[ServiceInfo] = &[
    ServiceInfo {
        name: "anthropic",
        env_vars: &["ANTHROPIC_API_KEY"],
        api_domains: &[
            "api.anthropic.com",
            "console.anthropic.com",
            "claude.ai",
            "mcp-proxy.anthropic.com",
        ],
    },
    ServiceInfo {
        name: "cursor",
        env_vars: &["CURSOR_API_KEY"],
        api_domains: &[
            "api2.cursor.sh",
            "api3.cursor.sh",
            "repo42.cursor.sh",
            "cursor.com",
        ],
    },
    ServiceInfo {
        name: "droid",
        env_vars: &["FACTORY_API_KEY"],
        api_domains: &["api.factory.ai", "app.factory.ai", "relay.factory.ai"],
    },
    ServiceInfo {
        name: "github",
        env_vars: &["GH_TOKEN", "GITHUB_TOKEN"],
        api_domains: &[
            "api.github.com",
            "github.com",
            "raw.githubusercontent.com",
            "gist.github.com",
            "copilot.github.com",
            "api.githubcopilot.com",
        ],
    },
    ServiceInfo {
        name: "google",
        env_vars: &["GOOGLE_API_KEY"],
        api_domains: &[
            "generativelanguage.googleapis.com",
            "oauth2.googleapis.com",
            "aiplatform.googleapis.com",
            "vertexai.googleapis.com",
        ],
    },
    ServiceInfo {
        name: "groq",
        env_vars: &["GROQ_API_KEY"],
        api_domains: &["api.groq.com"],
    },
    ServiceInfo {
        name: "mistral",
        env_vars: &["MISTRAL_API_KEY"],
        api_domains: &["api.mistral.ai"],
    },
    ServiceInfo {
        name: "nebius",
        env_vars: &["NEBIUS_API_KEY"],
        api_domains: &["api.studio.nebius.com", "api.tokenfactory.nebius.com"],
    },
    ServiceInfo {
        name: "openai",
        env_vars: &["OPENAI_API_KEY"],
        api_domains: &[
            "api.openai.com",
            "openai.com",
            "chatgpt.com",
            "www.chatgpt.com",
        ],
    },
    ServiceInfo {
        name: "openrouter",
        env_vars: &["OPENROUTER_API_KEY"],
        api_domains: &["openrouter.ai"],
    },
    ServiceInfo {
        name: "xai",
        env_vars: &["XAI_API_KEY"],
        api_domains: &["api.x.ai"],
    },
];

/// Look up a service by the name stored in a secret row.
pub fn service_info(name: &str) -> Option<&'static ServiceInfo> {
    SERVICES_INFO.iter().find(|info| info.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SERVICES;

    #[test]
    fn every_offered_service_has_environment_variables_and_domains() {
        for name in SERVICES {
            let info = service_info(name).unwrap_or_else(|| panic!("missing metadata for {name}"));
            assert!(!info.env_vars.is_empty(), "{name} has no env var");
            assert!(!info.api_domains.is_empty(), "{name} has no domains");
        }
        assert!(service_info("MY_TOKEN").is_none());
    }
}
