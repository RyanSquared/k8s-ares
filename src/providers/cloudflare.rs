// vim:set foldmethod=marker:

// starting doc {{{
//! A CloudFlare provider for ARES deployments.
//!
//! Configuration example:
//!
//! ```yaml
//! apiVersion: v1
//! kind: Secret
//! metadata:
//!   name: ares-secret
//! stringData:
//!   ares.yaml: |-
//!     - selector:
//!       - ***
//!       provider: cloudflare
//!       providerConfig:
//!         apiToken: ***
//! ---
//! apiVersion: v1
//! kind: Secret
//! metadata:
//!   name: ares-secret
//! stringData:
//!   ares.yaml: |-
//!     - selector:
//!       - ***
//!       provider: cloudflare
//!       providerConfig:
//!         email: ryan@***
//!         apiKey: ***
//! ```
// }}}

// {{{ imports
use anyhow::{anyhow, Result};
use serde::{Serialize, Deserialize};
use serde_json::value::{Value, Index, from_value};
use reqwest::header;

use super::util::{ProviderBackend, SubDomainName, FullDomainName, ZoneDomainName, Record};
use crate::reqwest_client_builder;
use crate::xpathable::XPathable;

use std::convert::{TryFrom, TryInto};
// }}}

static BASE_URL: &str = "https://api.cloudflare.com/client/v4";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum CloudFlareConfig {
    /// A CloudFlare API token. Unlike an API key (when combined with an email,
    /// gives full-account access), an API token can be limited to a specific
    /// zone, a specific set of zones, or a certain set of permissions.
    ///
    /// To set up an API Token, navigate to the "My Profile" section of the
    /// CloudFlare dashboard, then navigate to the "API Tokens" section. Then,
    /// click the "Create Token" button, and use the "Edit zone DNS" template.
    /// The required permissions are:
    ///
    /// - Zone / Zone / Read
    /// - Zone / DNS / Edit
    ///
    /// To limit your CloudFlare token to a specific zone, choose a zone from
    /// the Zone Resources option, which is already set up using the template.
    ///
    /// It is recommended to set the TTL to no more than a year. It is unknown
    /// whether or not CloudFlare will automatically notify users when a token
    /// is about to expire.
    Token {
        #[serde(rename="apiToken")]
        api_token: String,
    },
    /// A CloudFlare API Key. Unlike an API Token, this key - when combined
    /// with the email address of the account - is given the full permissions
    /// of the account.
    ///
    /// To get your API Key, navigate to the "My Profile" section of the
    /// CloudFlare dashboard, then use the "View" option for the Global API
    /// Key. You will usually be prompted to enter your password and solve a
    /// CAPTCHA.
    ///
    /// You will have to use your API Key in combination with the email
    /// associated with the account for API Key authentication.
    EmailKey {
        #[serde(rename="email")]
        email: String,
        #[serde(rename="apiKey")]
        api_key: String,
    },
}

macro_rules! client_builder {
    (auth::bearer(auth_token => $token:expr)) => ({
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION,
                       header::HeaderValue::from_str(format!("Bearer {}", $token).as_str())?);
        reqwest_client_builder!().default_headers(headers)
    });
    (auth::key(auth_email => $email:expr, auth_key => $key:expr)) => ({
        let mut headers = header::HeaderMap::new();
        let x_auth_email = header::HeaderName::from_static("x-auth-email");
        let x_auth_key = header::HeaderName::from_static("x-auth-key");
        headers.insert(x_auth_email, header::HeaderValue::from_str($email.as_str())?);
        headers.insert(x_auth_key, header::HeaderValue::from_str($key.as_str())?);
        reqwest_client_builder!().default_headers(headers)
    });
}

impl CloudFlareConfig {
    /// Get a Zone ID for a given domain name.
    async fn get_zone(&self, c: &reqwest::Client, zone: &ZoneDomainName) -> Result<String> {
        let result: Value = c.get(format!("{}/zones?name={}", BASE_URL, zone).as_str())
            .send().await?
            .json().await?;
        let zone_id = result
            .xpath("/result/0/id")?
            .as_str()
            .ok_or(anyhow!("Unable to convert zone ID to string"))?;
        Ok(zone_id.to_string())
    }

    /// Create a Reqwest client using the cloudflare::client_builder!().
    fn get_client(&self) -> Result<reqwest::Client> {
        match self {
            CloudFlareConfig::Token { api_token } => {
                Ok(client_builder!(auth::bearer(auth_token => api_token)).build()?)
            },
            CloudFlareConfig::EmailKey { email, api_key } => {
                Ok(client_builder!(auth::key(auth_email => email, auth_key => api_key)).build()?)
            }
        }
    }
}

#[async_trait::async_trait]
impl ProviderBackend for CloudFlareConfig {
    async fn get_zone(&self, domain: &FullDomainName) -> Result<ZoneDomainName> {
        // bubble up for every segment of the domain name
        // eventually we should hit a valid record
        let mut index = 0;
        let len = domain.len();
        let client = self.get_client()?;
        while index != len {
            let substr = &domain[index..len];
            let result: Value = client.get(format!("{}/zones?name={}", BASE_URL, substr).as_str())
                .send().await?
                .json().await?;
            // check for error
            if result.xpath("/success")?.as_bool()
                     .ok_or(anyhow!("Unable to convert success to bool"))? {
                return Ok(result
                    .xpath("/result/0/name")?
                    .as_str()
                    .ok_or(anyhow!("Unable to convert result.name to str"))?
                    .to_string());
            }
            if let Some(offset) = substr.find(".") {
                // increment offset to capture the period
                index += offset + 1;
            } else {
                break
            }
        }
        Err(anyhow!("Unable to find DNS Zone for: {}", domain))
    }

    async fn get_records(&self, domain: &ZoneDomainName, name: &SubDomainName) ->
            Result<Vec<Record>> {
        let client = self.get_client()?;
        // Get Zone ID
        let result: Value = client.get(format!("{}/zones?name={}", BASE_URL, domain).as_str())
            .send().await?
            .json().await?;
        let zone_id = result
            .xpath("/result/0/id")?
            .as_str()
            .ok_or(anyhow!("Unable to convert zone ID to string"))?;

        // Get Domain Name from Zone ID
        let result: Value = client.get(format!("{}/zones/{}/dns_records?name={}",
                                               BASE_URL, zone_id, name).as_str())
            .send().await?
            .json().await?;

        let record_count = result
            .xpath("/result_info/count")?
            .as_u64()
            .ok_or(anyhow!("Unable to convert result_info.count to u64"))?;

        let mut records: Vec<Record> = Vec::with_capacity(record_count as usize);
        // TODO: implement pagination

        for record in result
                .xpath("/result")?
                .as_array()
                .ok_or(anyhow!("Unable to convert result to array"))? {
            // try xpath impl
            records.push(Record::new(
                record
                    .xpath("/zone_name")?
                    .as_str()
                    .ok_or(anyhow!("Unable to convert record[].zone_name to str"))?.to_string(),
                record
                    .xpath("/name")?
                    .as_str()
                    .ok_or(anyhow!("Unable to convert record[].name to str"))?.to_string(),
                record
                    .xpath("/ttl")?
                    .as_u64()
                    .ok_or(anyhow!("Unable to convert result to u64"))?,
                from_value(record.xpath("/type")?.clone())?,
                record
                    .xpath("/content")?
                    .as_str()
                    .ok_or(anyhow!("Unable to convert record[].content to str"))?.into()
                    ));
        }

        Ok(records)
    }

    async fn get_all_records(&self, domain: &ZoneDomainName) ->
            Result<std::collections::HashMap<SubDomainName, Vec<Record>>> {
        // pass
        unimplemented!();
    }

    async fn _add_record(&self, domain: &ZoneDomainName, record: &Record) -> Result<()> {
        // pass
        let client = self.get_client()?;
        let zone_id = self.get_zone(&client, domain).await?;
        let url = format!("{}/zones/{}/dns_records", BASE_URL, zone_id);
        let mut data = std::collections::HashMap::<&str, serde_json::Value>::new();
        data.insert("type", serde_json::to_value(&record.record_type)?);
        data.insert("name", serde_json::to_value(&record.fqdn)?);
        data.insert("content", serde_json::to_value(&record.value)?);
        data.insert("ttl", serde_json::to_value(record.ttl)?);
        let result: Value = client.post(url.as_str())
            .json(&data)
            .send()
            .await?
            .json()
            .await?;
        if result.xpath("/success")?.as_bool()
                 .ok_or(anyhow!("Unable to convert success to bool"))? {
            Ok(())
        } else {
            if let Ok(error_object) = result.xpath("/errors/0/error_chain/0/message") {
                let error_str = error_object
                    .as_str()
                    .ok_or(anyhow!("Unable to convert errors/0/error_chain/0/message to str"))?;
                Err(anyhow!("{}", error_str))
            } else {
                let error_str = result
                    .xpath("/errors/0/message")?
                    .as_str()
                    .ok_or(anyhow!("Unable to convert errors/0/message to str"))?;
                Err(anyhow!("{}", error_str))
            }
        }
    }

    async fn _delete_record(&self, domain: &ZoneDomainName, record: &Record) -> Result<()> {
        // pass
        unimplemented!();
    }
}
