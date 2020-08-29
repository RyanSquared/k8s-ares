use anyhow::anyhow;
use serde::{Serialize, Deserialize};
use serde_json::value::{Value, Index, from_value};
use super::util::{ProviderBackend, SubDomainName, FullDomainName, ZoneDomainName, Record};
use crate::reqwest_client_builder;
use crate::xpathable::XPathable;

use std::convert::{TryFrom, TryInto};

static BASE_URL: &str = "https://api.cloudflare.com/client/v4";

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum CloudFlareConfig {
    Token {
        #[serde(rename="apiToken")]
        api_token: String,
    },
    EmailKey {
        #[serde(rename="email")]
        email: String,
        #[serde(rename="apiKey")]
        api_key: String,
    },
}

macro_rules! client_builder {
    (auth::bearer(auth_token => $token:expr)) => ({
        use reqwest::header;
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION,
                       header::HeaderValue::from_str(format!("Bearer {}", $token).as_str())?);
        reqwest_client_builder!().default_headers(headers)
    });
    (auth::key(auth_email => $email:expr, auth_key => $key:expr)) => ({
        use reqwest::headers;
        let mut headers = headers::HeaderMap::new();
        headers.insert(header::AUTHORIZATION,
                       concat!("Bearer ", $token))
        reqwest_client_builder!().default_headers(headers)
    });
}

impl CloudFlareConfig {
    fn get_client(&self) -> anyhow::Result<reqwest::Client> {
        match self {
            CloudFlareConfig::Token { api_token } => {
                Ok(client_builder!(auth::bearer(auth_token => api_token)).build()?)
            },
            CloudFlareConfig::EmailKey { api_key, email } => {
                unimplemented!("not yet!");
            }
        }
    }
}

#[async_trait::async_trait]
impl ProviderBackend for CloudFlareConfig {
    async fn get_records(&self, domain: ZoneDomainName, name: SubDomainName) ->
            anyhow::Result<Vec<Record>> {
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
                    .xpath("/name")?
                    .as_str()
                    .ok_or(anyhow!("Unable to convert record[].name to str"))?.into(),
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

    async fn get_all_records(&self, domain: ZoneDomainName) ->
            anyhow::Result<std::collections::HashMap<SubDomainName, Vec<Record>>> {
        // pass
        return Err(anyhow::anyhow!("NYI"));
    }

    async fn add_record(&mut self, domain: ZoneDomainName, record: Record) ->
            anyhow::Result<()> {
        // pass
        return Err(anyhow::anyhow!("NYI"));
    }

    async fn delete_record(&mut self, domain: ZoneDomainName, record: Record) ->
            anyhow::Result<()> {
        // pass
        return Err(anyhow::anyhow!("NYI"));
    }
}
