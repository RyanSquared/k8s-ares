use clap::Clap;

#[derive(Clap, Debug)]
#[clap(version="1.0", author="Ryan H. <ryan@hashbang.sh>")]
pub struct Opts {
    /// Name of Secret to load configuration from.
    #[clap(long, env="SECRET")]
    #[clap(default_value="ares-secret")]
    pub secret: String,

    /// Key of Secret to load configuration from.
    #[clap(long, env="SECRET_KEY")]
    #[clap(default_value="ares.yaml")]
    pub secret_key: String,

    /// Namespace where the Secret is stored.
    #[clap(long, env="SECRET_NAMESPACE")]
    #[clap(default_value="default")]
    pub secret_namespace: String,
}
