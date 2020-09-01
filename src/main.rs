#![warn(clippy::all, clippy::pedantic)]
#![warn(missing_docs)]
// vim:set et sw=4 ts=4 foldmethod=marker:

// starting doc {{{
//! ARES: Automatic REcord System.
//!
//! A Kubernetes-native system to automatically create and manage DNS records
//! meant to run in parallel with External DNS.
//!
//! Configuration is managed through the ares-secret Secret, typically in the
//! default namespace. This may change in the future to default to the
//! namespace that ARES is deployed in.
//!
//! ## Configuration
//!
//! A configuration file should look like this:
//!
//! ```yaml
//! - selector:
//!   - syntixi.io
//!   provider: cloudflare
//!   providerOptions:
//!     apiToken: ***
//! ```
//!
//! The corresponding Secret can look like:
//!
//! ```yaml
//! apiVersion: v1
//! kind: Secret
//! metadata:
//!   name: ares-secret
//! stringData:
//! - selector:
//!   - syntixi.io
//!   provider: cloudflare
//!   providerOptions:
//!     apiToken: ***
//! ```
//!
//! If you want to control multiple domain zones across multiple different
//! providers, you can add another element into the default array and
//! configure another provider there. You can configure multiple domain zones
//! through a single provider.
//!
//! ## Custom Resource Definitions
//!
//! ARES watches over the syntixi.io/v1alpha1/Record CustomResourceDefinition
//! to know which domain names to add, remove, or modify. Some examples of the
//! resource are below.
//!
//! ```yaml
//! apiVersion: syntixi.io/v1alpha1
//! kind: Record
//! metadata:
//!   name: example
//! spec:
//!   fqdn: example.syntixi.io
//!   ttl: 100
//!   type: CNAME
//!   values:
//!   - syntixi.io
//! ---
//! apiVersion: syntixi.io/v1alpha1
//! kind: Record
//! metadata:
//!   name: internal
//! spec:
//!   fqdn: internal.syntixi.io
//!   ttl: 100
//!   type: A
//!   values:
//!   - 10.0.23.1
//! ---
//! apiVersion: syntixi.io/v1alpha1
//! kind: Record
//! metadata:
//!   name: roundrobin
//! spec:
//!   fqdn: rr.syntixi.io
//!   ttl: 100
//!   type: AAAA
//!   values:
//!   - 2600:8803:7881:1000:6c82:9131:dead:beef
//!   - 2600:8803:7881:1000:6c82:9131:1c3:d00d
//!   - 2600:8803:7881:1000:6c82:9131:c0:ffee
//! ```
//!
//! When a syntixi.io/v1alpha1/Record resource is created, an additional record
//! is made for ARES to track ownership over the DNS record. So long as that
//! tracking record exists, when the Kubernetes resource is deleted, the
//! corresponding record and tracking record will be deleted.
// }}}

// imports {{{
use clap::Clap;

use std::ops::Deref;

use slog::{
    crit, debug, error, info, log, o,
    Drain, Logger,
};

use anyhow::anyhow;

use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::{Event, Secret};
use kube::{
    api::{Api, ListParams, Meta},
    Client,
};
use kube_runtime::{utils::try_flatten_applied, watcher};
use kube_derive::{CustomResource};

mod cli;

mod xpathable;

mod providers;
mod program_config;
mod record_spec;

use program_config::AresConfig;
use providers::{ProviderConfig, util::{ProviderBackend, ZoneDomainName,
                                       RecordType, Record as RecordObject}};
use record_spec::{Record, RecordValueCollector};
// }}}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts: cli::Opts = cli::Opts::parse();
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let root_logger = slog::Logger::root(
        drain,
        o!("secret" => opts.secret.clone(),
           "secret_key" => opts.secret_key.clone(),
           "secret_namespace" => opts.secret_namespace.clone()),
    );
    let client = Client::try_default().await?;

    info!(root_logger, "Loading configuration from Secret");
    let secrets: Api<Secret> = Api::namespaced(client, opts.secret_namespace.as_str());
    let secret = secrets.get(opts.secret.as_str()).await?;
    let config_data = secret
        .data
        .ok_or(anyhow!("Unable to get data from Secret"))?;
    let config_content = config_data
        .get(opts.secret_key.as_str())
        .ok_or(anyhow!("Unable to get key from Secret"))?
        .clone().0;

    let mut config: Vec<AresConfig> = serde_yaml::from_str(std::str::from_utf8(&config_content[..])?)?;

    { // Testing RecordSpec
        let records: Api<Record> = Api::all(Client::try_default().await?);
        let list = records.list(&ListParams::default()).await?;
        let record = list.iter().next().unwrap();
        if let Some(collector_obj) = &record.spec.value_from {
            let collector = collector_obj.deref();
            let zone = String::from("syntixi.io");
            let mut config = config.remove(0).provider;
            let mut builder = RecordObject::builder(record.spec.fqdn.clone(), zone, RecordType::A);
            collector.on_value_change(&record.metadata, &mut config, &mut builder).await?;
        }
    }

    // normally, we'd have a watcher over a CRD, but we're just gonna oneshot the match
    for backend in config {
        let provider: &dyn ProviderBackend = backend.provider.deref();

        for selector in &backend.selector {
            info!(root_logger, "found selector"; o!("selector" => selector));
            let records = provider.get_records(selector.into(), selector.into()).await?;
            for record in records {
                info!(root_logger, "have record! is {:?}", record);
            }
        }
    }

    Ok(())
}
