use actix_web::{web, App, HttpServer};
use configuration::{Configuration, FSyncStrategy};
use log::{error, info};
use node_reader::NodeReader;
use reqwest::blocking::Client;
use routes::*;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::{mpsc, RwLock};
use std::thread;

mod cache;
mod configuration;
mod node_reader;
mod routes;
mod tree_node;

const CONFIGURATION_PATH: &str = "config.json";

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let configuration = Configuration::read(CONFIGURATION_PATH).unwrap();
    let use_strict_fsync = configuration.fsync() == FSyncStrategy::Strict;

    info!("Listening on port: {}", configuration.port());
    info!("Use fsynch strict: {}", use_strict_fsync);
    info!("Cache size: {}MB", configuration.cache_size() / 1024 / 1024);
    info!(
        "Max range response: {:#?}",
        match configuration.max_range_response() {
            Some(v) => v.to_string(),
            None => "Not set".to_string(),
        }
    );

    let path = Path::new("data").to_path_buf();
    create_data_directory(&path).expect("Failed to create data directory");

    let mut store = NodeReader::new(
        path,
        configuration.cache_size(),
        configuration.max_range_response(),
        configuration.fsync() == FSyncStrategy::Strict,
    )
    .expect("Failed to create NodeReader");

    info!("Starting sanity check");
    store.sanity_check().unwrap();
    info!("Sanity check completed");

    info!("Starting service: ...");
    let (tx, rx) = mpsc::channel::<WriteEvent>();
    let replicas = Arc::new(configuration.replicas().clone());

    thread::spawn(move || event_listener(rx, replicas));

    if configuration.is_replica() {
        start_replica(configuration, store, tx).await
    } else {
        start_main(configuration, store, tx).await
    }
}

async fn start_main(
    configuration: Configuration,
    node_reader: NodeReader,
    tx: Sender<WriteEvent>,
) -> Result<(), std::io::Error> {
    let store = Arc::new(RwLock::new(node_reader));

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(store.clone()))
            .app_data(web::Data::new(AtomicUsize::new(0)))
            .app_data(web::Data::new(tx.clone()))
            .service(get)
            .service(get_range)
            .service(insert)
            .service(bulk_insert)
            .service(delete)
    })
    .bind(("::", configuration.port()))?
    .run()
    .await
}

async fn start_replica(
    configuration: Configuration,
    node_reader: NodeReader,
    tx: Sender<WriteEvent>,
) -> Result<(), std::io::Error> {
    let store = Arc::new(RwLock::new(node_reader));
    let public_store = store.clone();
    let public = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(public_store.clone()))
            .app_data(web::Data::new(AtomicUsize::new(0)))
            .service(get)
            .service(get_range)
    })
    .bind(("::", configuration.port()))?
    .run();

    let replication = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(store.clone()))
            .app_data(web::Data::new(AtomicUsize::new(0)))
            .app_data(web::Data::new(tx.clone()))
            .service(insert)
            .service(bulk_insert)
            .service(delete)
    })
    .bind(("::", configuration.replication_port()))?
    .run();

    tokio::join!(public, replication).0
}

fn create_data_directory(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        fs::create_dir_all(&path)?;
        info!("Directory created: {}", path.to_str().unwrap());
    } else {
        info!("Directory already exists: {}", path.to_str().unwrap());
    }

    Ok(())
}

#[derive(Debug)]
enum WriteEvent {
    BulkInsert(HashMap<String, String>),
    Insert(String, String),
    Delete(String),
}

fn event_listener(rx: mpsc::Receiver<WriteEvent>, replicas: Arc<Vec<String>>) {
    info!("[Event Listener] Started event listener");
    if replicas.len() == 0 {
        for _ in rx {}
    } else {
        for r in &*replicas {
            info!("[Event Listener] Replica: {r}");
        }

        let client = Client::new();

        for received in rx.iter() {
            for replica in &*replicas {
                let result = match received {
                    WriteEvent::Insert(ref key, ref value) => {
                        let mut url = replica.clone();
                        url.push_str(&format!("/kv/{key}"));
                        client.post(url).json(value).send()
                    }
                    WriteEvent::BulkInsert(ref entries) => {
                        let mut url = replica.clone();
                        url.push_str(&format!("/bulk"));
                        client.post(url).json(&entries).send()
                    }
                    WriteEvent::Delete(ref key) => {
                        let mut url = replica.clone();
                        url.push_str(&format!("/kv/{key}"));
                        client.delete(url).send()
                    }
                };

                match result {
                    Ok(r) if r.status().is_success() => {}
                    _ => {
                        error!("Failed to send message to replica")
                    }
                }
            }
        }
    }
}
