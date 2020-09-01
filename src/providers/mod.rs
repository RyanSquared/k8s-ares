// vim:set et sw=4 ts=4 foldmethod=marker:

// {{{ imports
use serde::{Serialize, Deserialize};

pub mod cloudflare;
// }}}

pub mod util { // {{{
    use anyhow::{anyhow, Result};

    use serde::{Serialize, Deserialize};
    pub type ZoneDomainName = String;
    pub type FullDomainName = String;
    pub type SubDomainName = String;

    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub enum RecordType {
        // Standard
        A,
        AAAA,
        ALIAS,
        CNAME,
        MX,
        NS,
        PTR,
        SOA,
        SRV,
        TXT,
        // DNSSEC types
        DNSKEY,
        DS,
        NSEC,
        NSEC3,
        NSEC3PARAM,
        RRSIG,
    }

    #[derive(Serialize, Deserialize, Debug)]
    pub struct Record {
        pub fqdn: FullDomainName,
        pub zone: ZoneDomainName,
        pub record_type: RecordType,
        pub ttl: u64,
        pub value: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub struct RecordBuilder {
        pub fqdn: FullDomainName,
        pub zone: ZoneDomainName,
        pub record_type: RecordType,
        pub ttl: Option<u64>,
        pub value: Option<String>,
    }

    impl Record {
        pub fn new(zone: ZoneDomainName, fqdn: FullDomainName, ttl: u64,
                   _type: RecordType, value: String) -> Record {
            Record {
                fqdn: fqdn,
                zone: zone,
                ttl: ttl,
                record_type: _type,
                value: value,
            }
        }

        pub fn builder(fqdn: FullDomainName,
                       zone: ZoneDomainName,
                       record_type: RecordType) -> RecordBuilder {
            RecordBuilder {
                fqdn: fqdn,
                zone: zone,
                record_type: record_type,
                ttl: None,
                value: None,
            }
        }
    }

    impl RecordBuilder {
        pub fn value(&mut self, value: String) -> &mut Self {
            self.value = Some(value);
            self
        }

        pub fn ttl(&mut self, ttl: u64) -> &mut Self {
            self.ttl = Some(ttl);
            self
        }

        pub fn try_build(self) -> Result<Record> {
            let ttl = self.ttl.ok_or(anyhow!("Missing TTL"))?;
            let value = self.value.ok_or(anyhow!("Missing value"))?;
            Ok(Record::new(self.zone,
                           self.fqdn,
                           ttl,
                           self.record_type,
                           value))
        }
    }

    /// Generate a Reqwest client for use in Providers. Providers that
    /// implement an authentication logic should build their clients using a
    /// custom client_builder!() macro for each provider and, if necessary,
    /// create a get_client() function that can perform any necessary
    /// handshakes.
    #[macro_export]
    macro_rules! reqwest_client_builder {
        () => ({
            reqwest::Client::builder()
                .cookie_store(true)
                .user_agent(concat!(
                    env!("CARGO_PKG_NAME"),
                    "/",
                    env!("CARGO_PKG_VERSION"),
                ))
        });
    }

    /// `ProviderBackend` is a generic implementation for all potential
    /// DNS backends, and can be used as a dynamic trait object to implement
    /// interactions with the dynamic backend.
    #[async_trait::async_trait]
    pub trait ProviderBackend: Send {
        /// Get a deployed record from the backend service.
        async fn get_records(&self, domain: &ZoneDomainName, name: &SubDomainName) ->
                anyhow::Result<Vec<Record>>;

        /// Get all records from the backend service, as a pairing of record entry
        /// to record value.
        async fn get_all_records(&self, domain: &ZoneDomainName) ->
                anyhow::Result<std::collections::HashMap<SubDomainName, Vec<Record>>>;

        /// Add a DNS Record.
        async fn add_record(&mut self, domain: &ZoneDomainName, record: &Record) ->
                anyhow::Result<()>;

        /// Delete a DNS Record.
        async fn delete_record(&mut self, domain: &ZoneDomainName, record: &Record) ->
                anyhow::Result<()>;
    }
} // }}}

use util::ProviderBackend;
use cloudflare::CloudFlareConfig as CloudFlare;

trait_enum::trait_enum! {
    #[derive(Serialize, Deserialize, Debug)]
    #[serde(tag="provider", content="providerOptions")]
    pub enum ProviderConfig: ProviderBackend {
        #[serde(rename="cloudflare")]
        CloudFlare,
    }
}
