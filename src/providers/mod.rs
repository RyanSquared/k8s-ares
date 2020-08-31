// vim:set et sw=4 ts=4 foldmethod=marker:

// {{{ imports
use serde::{Serialize, Deserialize};

pub mod cloudflare;
// }}}

pub mod util {
    use serde::{Serialize, Deserialize};
    pub type ZoneDomainName = String;
    pub type FullDomainName = String;
    pub type SubDomainName = String;

    #[derive(Serialize, Deserialize, Debug)]
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
        NSEC3PAARAM,
        RRSIG,
    }

    #[derive(Serialize, Deserialize, Debug)]
    pub struct Record {
        fqdn: FullDomainName,
        ttl: u64,
        record_type: RecordType,
        value: String,
    }

    impl Record {
        pub fn new(fqdn: FullDomainName, ttl: u64, _type: RecordType, value: String) -> Record {
            Record {
                fqdn: fqdn,
                ttl: ttl,
                record_type: _type,
                value: value,
            }
        }
    }

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

    /// Generic trait for getting, setting, and deleting DNS records.
    #[async_trait::async_trait]
    pub trait ProviderBackend {
        /// Get a deployed record from the backend service.
        async fn get_records(&self, domain: ZoneDomainName, name: SubDomainName) ->
                anyhow::Result<Vec<Record>>;

        /// Get all records from the backend service, as a pairing of record entry
        /// to record value.
        async fn get_all_records(&self, domain: ZoneDomainName) ->
                anyhow::Result<std::collections::HashMap<SubDomainName, Vec<Record>>>;

        /// Add a DNS Record.
        async fn add_record(&mut self, domain: ZoneDomainName, record: Record) ->
                anyhow::Result<()>;

        /// Delete a DNS Record.
        async fn delete_record(&mut self, domain: ZoneDomainName, record: Record) ->
                anyhow::Result<()>;
    }
}

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
