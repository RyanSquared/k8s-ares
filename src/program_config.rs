// vim:set et sw=4 ts=4 foldmethod=marker:

// imports {{{
use serde::{Serialize, Deserialize};

use super::providers::ProviderConfig;
// }}}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all(serialize="camelCase", deserialize="camelCase"))]
pub struct AresConfig {
    pub selector: Vec<String>,

    #[serde(flatten)]
    pub provider: ProviderConfig,
}

impl AresConfig {
    /// Iterate over Selectors and ensure that a given item matches at least
    /// one of the Selectors. The Selector syntax must be a raw string, not
    /// something like a regex pattern. To match subdomains under example.com
    /// but not example.com itself, use the selector ".example.com", then have
    /// a Selector for another AresConfig (further down the chain) that matches
    /// "example.com".
    pub fn matches_selector(&self, item: &str) -> bool {
        self.selector.iter().filter(|x| item.ends_with(x.as_str())).next().is_some()
    }
}
