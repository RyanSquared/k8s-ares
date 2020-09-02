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
        pub fn value(self, value: String) -> Self {
            RecordBuilder {
                value: Some(value),
                ..self
            }
        }

        pub fn ttl(self, ttl: u64) -> Self {
            RecordBuilder {
                ttl: Some(ttl),
                ..self
            }
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
    pub trait ProviderBackend: Send + Sync {
        /// Obtaina a Zone for a DNS Record; this usually results in a batch attempt
        /// of obtaining zone information from the server, so do not call it more
        /// than is required.
        async fn get_zone(&self, domain: &FullDomainName) -> Result<ZoneDomainName>;

        /// Get a deployed record from the backend service.
        async fn get_records(&self, domain: &ZoneDomainName, name: &FullDomainName) ->
                Result<Vec<Record>>;

        /// Get all records from the backend service, as a pairing of record entry
        /// to record value.
        async fn get_all_records(&self, domain: &ZoneDomainName) ->
                Result<std::collections::HashMap<SubDomainName, Vec<Record>>>;

        /// Add a DNS Record.
        async fn _add_record(&self, domain: &ZoneDomainName, record: &Record) -> Result<()>;

        /// Delete a DNS Record.
        async fn _delete_record(&self, domain: &ZoneDomainName, record: &Record) -> Result<()>;

        /// Add a DNS record and tracking record.
        async fn add_record(&self, domain: &ZoneDomainName, record: &Record) -> Result<()> {
            // TODO more heritage information in DNS record
            let tracking_domain = format!("{}.{}", "_owner", &record.fqdn);
            let tracking_record = self
                .get_records(domain, &tracking_domain)
                .await?;
            if let Some(r) = tracking_record.get(0) {
                // we have a tracking record, we should *not* have a tracking record.
                return Err(anyhow!("Found existing tracking record: {}", tracking_domain));
            }
            let record_builder = Record::builder(tracking_domain, domain.clone(),
                                                 RecordType::TXT)
                .value("ares".to_string())
                .ttl(1);
            self._add_record(domain, &record_builder.try_build()?).await?;
            self._add_record(domain, record).await?;
            Ok(())
        }

        /// Remove a DNS record and tracking record.
        async fn delete_record(&self, domain: &ZoneDomainName, record: &Record) ->
                Result<()> {
            let tracking_domain = format!("{}.{}", "_owner", &record.fqdn);
            let tracking_record = self
                .get_records(domain, &tracking_domain)
                .await?;
            match tracking_record.iter().filter(|x| x.value == "ares".to_string()).next() {
                Some(r) => {
                    self._delete_record(domain, record).await?;
                    self._delete_record(domain, r).await?;
                    Ok(())
                },
                None => Err(anyhow!("Missing tracking record: {}", tracking_domain))
            }
        }


        /// Get records from the remote server and ensure that the remote records
        /// match the given records.
        async fn sync_records(&self, record_builder: &RecordBuilder,
                              records: &Vec<String>) -> Result<()> {
            let fqdn = &record_builder.fqdn;
            let zone = &record_builder.zone;
            let remote_records = self.get_records(zone, fqdn).await?;
            for record in remote_records.iter().filter(|x| !records.contains(&x.value)) {
                self.delete_record(zone, record).await?;
            }
            for record in records {
                if remote_records.iter().filter(|x| x.value == *record).next().is_none() {
                    let record_entry = record_builder
                        .clone()
                        .value(record.clone())
                        .ttl(1) // TODO: custom TTL
                        .try_build()?;
                    self.add_record(zone, &record_entry).await?;
                }
            }
            Ok(())
        }
    }
} // }}}

use util::ProviderBackend;
use cloudflare::CloudFlareConfig as CloudFlare;

trait_enum::trait_enum! {
    #[derive(Serialize, Deserialize, Clone, Debug)]
    #[serde(tag="provider", content="providerOptions")]
    pub enum ProviderConfig: ProviderBackend {
        #[serde(rename="cloudflare")]
        CloudFlare,
    }
}
