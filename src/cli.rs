use clap::Clap;

#[derive(Clap, Debug)]
#[clap(version = "1.0", author = "Ryan H. <ryan@hashbang.sh>")]
pub struct Opts {
    #[clap(long, env="SECRET")]
    #[clap(default_value="ares-secret")]
    #[clap(help="Name of Secret to load configuration from.")]
    pub secret: String,

    #[clap(long, env="SECRET_KEY")]
    #[clap(default_value="ares.yaml")]
    #[clap(help="Key of SECRET to load configuration from.")]
    pub secret_key: String,

    #[clap(long, env="SECRET_NAMESPACE")]
    #[clap(default_value="default")]
    #[clap(help="Namespace where the Secret is stored.")]
    pub secret_namespace: String,

    #[clap(long, env="POD_NAMESPACE")]
    #[clap(default_value="default")]
    #[clap(help="Namespace where Pods are stored for PodSelectors")]
    pub pod_namespace: String,
}
