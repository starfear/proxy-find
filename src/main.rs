use clap::{App as ClapApp, Arg as ClapArg};
use futures::stream::TryStreamExt;
use reqwest::{Error as ReqwestError, Proxy};
use std::{error::Error, sync::Arc, thread::sleep as thread_sleep, time::Duration};
use tokio::{
    fs,
    io::{BufReader, Error as IoError},
    prelude::*,
    runtime::Builder as RuntimeBuilder,
    sync::Mutex,
};

struct AppConfig {
    pub target: Box<str>,
    pub input: Box<str>,
    pub output: Box<str>,
    pub cores: usize,
    pub delay: u64,
    pub timeout: u64,
}

fn main() -> Result<(), Box<dyn Error>> {
    // build a config
    let matches = ClapApp::new("proxy-finder")
        .author("q-chan")
        .about("Simple utility for checking status of proxies by list")
        .arg(
            ClapArg::with_name("target")
                .long("target")
                .short("h")
                .help("Target ip or website to test if proxy can connect other ips")
                .default_value("https://ifconfig.me")
                .takes_value(true),
        )
        .arg(
            ClapArg::with_name("output")
                .long("output")
                .short("o")
                .help("Output path of file where valid proxies will be saved")
                .default_value("valid.txt")
                .takes_value(true),
        )
        .arg(
            ClapArg::with_name("input")
                .long("input")
                .short("i")
                .help("Input path of file where proxies to check are located")
                .required(true)
                .takes_value(true),
        )
        .arg(
            ClapArg::with_name("cons_per_sec")
                .long("cps")
                .help("Amount of connections per seconds")
                .takes_value(true),
        )
        .arg(
            ClapArg::with_name("cores")
                .long("cores")
                .short("c")
                .help("Amount of cores used")
                .takes_value(true),
        )
        .arg(
            ClapArg::with_name("timeout")
                .long("timeout")
                .short("m")
                .help("Max timeout(secs) of request")
                .takes_value(true),
        )
        .get_matches();

    let cfg = Arc::new(AppConfig {
        target: matches.value_of("target").unwrap().into(),
        input: matches.value_of("input").unwrap().into(),
        output: matches.value_of("output").unwrap().into(),
        delay: 1000
            / matches
                .value_of("cons_per_sec")
                .map(|x| x.parse().expect("Invalid amount of connections"))
                .unwrap_or(30),
        cores: matches
            .value_of("cores")
            .map(|x| x.parse().expect("Invalid amount of cores"))
            .unwrap_or(num_cpus::get()),
        timeout: matches
            .value_of("timeout")
            .map(|x| x.parse().expect("Invalid timeout"))
            .unwrap_or(5),
    });

    // build an runtime
    let mut runtime = RuntimeBuilder::new()
        .threaded_scheduler()
        .core_threads(cfg.cores)
        .thread_name("worker")
        .thread_stack_size(3 * 1024 * 1024)
        .enable_all()
        .build()?;

    // read proxies
    let proxies = runtime.block_on(load_list(&cfg.input))?;

    // create valid file
    let valid_file = Arc::new(Mutex::new(
        runtime.block_on(fs::File::create(cfg.output.to_string()))?,
    ));

    let len = proxies.len();
    for (idx, proxy) in proxies.into_iter().enumerate() {
        println!("{}%", (idx as f32 / len as f32 * 100.0) as u32);
        runtime.spawn(process_proxy(
            Arc::clone(&cfg),
            Arc::clone(&valid_file),
            proxy,
        ));

        thread_sleep(Duration::from_millis(cfg.delay));
    }

    println!(
        "Waiting `{}` seconds to process all requests...",
        cfg.timeout
    );
    thread_sleep(Duration::from_secs((cfg.timeout as f32 * 1.1) as u64));

    Ok(())
}

// should be rewritten (using custom reqwest fork...)
#[inline]
async fn check_proxy(target: &str, timeout: u64, proxy: &str) -> Result<bool, ReqwestError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .proxy(Proxy::all(proxy)?)
        .build()?;

    let resp = client.get(target).send().await;

    println!("{:?}", resp);

    Ok(resp.is_ok())
}

enum ProcessProxyError {
    Reqwest(ReqwestError),
    Io(IoError),
}

impl From<ReqwestError> for ProcessProxyError {
    fn from(err: ReqwestError) -> Self {
        ProcessProxyError::Reqwest(err)
    }
}

impl From<IoError> for ProcessProxyError {
    fn from(err: IoError) -> Self {
        ProcessProxyError::Io(err)
    }
}

#[inline]
async fn process_proxy(
    cfg: Arc<AppConfig>,
    valid_file: Arc<Mutex<fs::File>>,
    proxy: String,
) -> Result<(), ProcessProxyError> {
    if check_proxy(cfg.target.as_ref(), cfg.timeout, &proxy).await? {
        valid_file
            .lock()
            .await
            .write_all([&proxy, "\n"].concat().as_bytes())
            .await?;
    }

    Ok(())
}

#[inline]
async fn load_list(path: &str) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(BufReader::new(fs::File::open(path).await?)
        .lines()
        .try_collect::<Vec<_>>()
        .await?)
}
