// vim:set et sw=4 ts=4 foldmethod=marker:

// imports {{{
use serde::{Serialize, Deserialize};

use super::providers::ProviderConfig;
// }}}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all(serialize="camelCase", deserialize="camelCase"))]
pub struct AresConfig {
    pub selector: Vec<String>,

    #[serde(flatten)]
    pub provider: ProviderConfig,
}
