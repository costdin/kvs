use crate::node_reader::NodeReader;
use crate::tree_node::TrieError;
use crate::WriteEvent;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, RwLock,
};

use actix_web::{
    delete, error, get, post,
    web::{self, Json},
    Result,
};

#[derive(Debug, Deserialize)]
pub struct RangeParameters {
    start_key: String,
    end_key: String,
}

#[get("/kv/{key}")]
async fn get(
    path: web::Path<String>,
    store: web::Data<Arc<RwLock<NodeReader>>>,
    counter: web::Data<AtomicUsize>,
) -> Result<Json<String>> {
    let key = path.into_inner();
    counter.fetch_add(1, Ordering::SeqCst);

    match store.write() {
        Ok(mut store) => to_json(store.get(&key)),
        Err(_) => Err(error::ErrorInternalServerError("")),
    }
}

#[post("/kv/{key}")]
async fn insert(
    path: web::Path<String>,
    body: web::Json<String>,
    store: web::Data<Arc<RwLock<NodeReader>>>,
    channel: web::Data<Sender<WriteEvent>>,
    counter: web::Data<AtomicUsize>,
) -> Result<()> {
    let key = path.into_inner();
    let value = body.into_inner();
    let sender = channel.into_inner();
    counter.fetch_add(1, Ordering::SeqCst);

    match store.write() {
        Ok(mut store) => send_event(
            sender,
            to_empty(store.insert(key.clone(), value.clone())),
            WriteEvent::Insert(key, value),
        ),
        Err(_) => Err(error::ErrorInternalServerError("")),
    }
}

#[delete("/kv/{key}")]
async fn delete(
    path: web::Path<String>,
    store: web::Data<Arc<RwLock<NodeReader>>>,
    channel: web::Data<Sender<WriteEvent>>,
    counter: web::Data<AtomicUsize>,
) -> Result<()> {
    let key = path.into_inner();
    let sender = channel.into_inner();
    counter.fetch_add(1, Ordering::SeqCst);

    match store.write() {
        Ok(mut store) => send_event(
            sender,
            to_empty(store.delete(key.clone())),
            WriteEvent::Delete(key),
        ),
        Err(_) => Err(error::ErrorInternalServerError("")),
    }
}

#[get("/bulk/range")]
async fn get_range(
    range_params: web::Query<RangeParameters>,
    store: web::Data<Arc<RwLock<NodeReader>>>,
    counter: web::Data<AtomicUsize>,
) -> Result<Json<Vec<(String, String)>>> {
    let RangeParameters { start_key, end_key } = range_params.into_inner();
    counter.fetch_add(1, Ordering::SeqCst);

    match store.write() {
        Ok(mut store) => to_json(store.get_range(&start_key, &end_key)),
        Err(_) => Err(error::ErrorInternalServerError("")),
    }
}

#[post("/bulk")]
async fn bulk_insert(
    request_body: web::Json<HashMap<String, String>>,
    store: web::Data<Arc<RwLock<NodeReader>>>,
    channel: web::Data<Sender<WriteEvent>>,
    counter: web::Data<AtomicUsize>,
) -> Result<()> {
    let entries = request_body.into_inner();
    let sender = channel.into_inner();
    counter.fetch_add(1, Ordering::SeqCst);

    match store.write() {
        Ok(mut store) => send_event(
            sender,
            to_empty(store.bulk_insert(entries.clone())),
            WriteEvent::BulkInsert(entries),
        ),
        Err(_) => Err(error::ErrorInternalServerError("")),
    }
}

fn send_event<T>(
    channel: Arc<Sender<WriteEvent>>,
    result: Result<T>,
    event: WriteEvent,
) -> Result<T> {
    match result {
        Ok(_) => {
            if let Err(e) = channel.send(event) {
                log::error!("Error while sending event: {:#?}", e);
            }
        }
        Err(_) => {}
    }

    result
}

fn to_empty<T>(result: Result<T, TrieError>) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(process_error(e)),
    }
}

fn to_json<T>(result: Result<T, TrieError>) -> Result<Json<T>> {
    match result {
        Ok(r) => Ok(web::Json(r)),
        Err(e) => Err(process_error(e)),
    }
}

fn process_error(e: TrieError) -> actix_web::Error {
    match e {
        TrieError::KeyError => error::ErrorBadRequest("Invalid key"),
        TrieError::ValueError => error::ErrorBadRequest("Invalid value"),
        TrieError::NotFound => error::ErrorBadRequest("Key not found"),
        _ => error::ErrorInternalServerError(""),
    }
}
