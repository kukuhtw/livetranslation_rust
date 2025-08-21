// src/main.rs
/*
=============================================================================
Project : Live Translation Rust — finish speaking, instantly translated. Powered by Rust for speed.
Author : Kukuh Tripamungkas Wicaksono (Kukuh TW)
Email : kukuhtw@gmail.com
WhatsApp : https://wa.me/628129893706
LinkedIn : https://id.linkedin.com/in/kukuhtw
=============================================================================/

*/

use std::{
    convert::Infallible,
    env,
    net::SocketAddr,
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::{Html, IntoResponse},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use base64::Engine as _;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::{broadcast::{self, Sender}, Mutex},
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_tungstenite::tungstenite::{self, handshake::client::generate_key};
use rustls::crypto::{CryptoProvider, ring};


use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};

use tracing_subscriber::EnvFilter;

use uuid::Uuid;


use tracing::{error, info};
use tokio::time::timeout;
use axum::http::HeaderMap;




// helper untuk log header tanpa bocor token
fn redact_headers(h: &HeaderMap) -> Vec<(String,String)> {
    h.iter().map(|(k,v)| {
        let ks = k.as_str().to_string();
        let vs = v.to_str().unwrap_or("<bin>").to_string();
        let val = if ks.eq_ignore_ascii_case("authorization") {
            "<redacted>".to_string()
        } else { vs };
        (ks, val)
    }).collect()
}

#[derive(Clone)]
struct AppState {
    rooms: Arc<DashMap<String, Sender<String>>>,
    base_url: String,
    api_key: String,
    model: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct CreateRoomReq {
    name: Option<String>,
}

#[derive(Serialize)]
struct CreateRoomResp {
    room_id: String,
    share_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    CryptoProvider::install_default(ring::default_provider())
    .expect("install rustls ring provider");

    let filter = EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new("info,axum=info,tower_http=info,live_translate=debug"));

    tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_max_level(tracing::Level::DEBUG)
    .with_target(false)
    .with_writer(std::io::stdout)
    .init();

tracing::info!("tracing initialized ✅");

    let api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");
    let model = env::var("REALTIME_MODEL").unwrap_or_else(|_| "gpt-4o-realtime-preview".to_string());
    let base_url = env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let port: u16 = env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8080);

    let state = AppState {
        rooms: Arc::new(DashMap::new()),
        base_url,
        api_key,
        model,
    };

    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("../static/index.html")) }))
        .route("/view", get(|| async { Html(include_str!("../static/view.html")) }))
        .route("/api/room", post(create_room))
        .route("/sse/:room", get(sse_room))
        .route("/ws/:room", get(ws_speaker))
        .nest_service("/static", ServeDir::new("static"))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::new().allow_methods(Any).allow_headers(Any).allow_origin(Any))
        .with_state(state);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = TcpListener::bind(addr).await?;
    println!("Open http://127.0.0.1:{port}");
    info!("listening on {}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn create_room(
    State(state): State<AppState>,
    Json(_req): Json<CreateRoomReq>,
) -> Json<CreateRoomResp> {
    let room_id = Uuid::new_v4().to_string();
    let (tx, _rx) = broadcast::channel::<String>(256);
    state.rooms.insert(room_id.clone(), tx);

    let share_url = format!("{}/view?room={}", state.base_url, room_id);
    info!("room created {} -> {}", room_id, share_url);
    Json(CreateRoomResp { room_id, share_url })
}

async fn sse_room(Path(room): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
    let tx = match state.rooms.get(&room) {
        Some(t) => t.clone(),
        None => return (StatusCode::NOT_FOUND, "room not found").into_response(),
    };

    let rx = tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| async move {
        match msg {
            Ok(s) => Some(Ok::<_, Infallible>(Event::default().data(s))),
            Err(_) => None,
        }
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ClientMsg {
    #[serde(rename = "init")]
    Init { name: String, pair: String },
    #[serde(rename = "commit")]
    Commit,
}

async fn ws_speaker(
    ws: WebSocketUpgrade,
    Path(room): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, room, state))
}

fn json_instr(src_name: &str, tgt_label_native: &str, name: &str) -> String {
    format!(
      "You are a real-time translator for {name}. \
       First transcribe in {src}. Then translate to {tgt} only. \
       Respond EXACTLY one JSON: {{\"src\":\"<{src} transcript>\",\"tgt\":\"<{tgt} translation>\"}}. \
       If {tgt} is 日本語, use Japanese script (かな/漢字), no romaji, no English.",
      src = src_name, tgt = tgt_label_native, name = name
    )
}
fn instructions_for(pair: &str, name: &str) -> (&'static str, String) {
    match pair {
        // existing
        "id-ja" => ("id", json_instr("Indonesian", "日本語", name)),
        "ja-id" => ("ja", json_instr("日本語", "Indonesian", name)),
        "id-en" => ("id", json_instr("Indonesian", "English", name)),
        "en-id" => ("en", json_instr("English", "Indonesian", name)),

        // Korean
        "id-ko" => ("id", json_instr("Indonesian", "한국어", name)),
        "ko-id" => ("ko", json_instr("한국어", "Indonesian", name)),

        // Arabic
        "id-ar" => ("id", json_instr("Indonesian", "العربية", name)),
        "ar-id" => ("ar", json_instr("العربية", "Indonesian", name)),

        // German
        "id-de" => ("id", json_instr("Indonesian", "Deutsch", name)),
        "de-id" => ("de", json_instr("Deutsch", "Indonesian", name)),

        // French
        "id-fr" => ("id", json_instr("Indonesian", "Français", name)),
        "fr-id" => ("fr", json_instr("Français", "Indonesian", name)),

        // Dutch
        "id-nl" => ("id", json_instr("Indonesian", "Nederlands", name)),
        "nl-id" => ("nl", json_instr("Nederlands", "Indonesian", name)),

        // Russian
        "id-ru" => ("id", json_instr("Indonesian", "Русский", name)),
        "ru-id" => ("ru", json_instr("Русский", "Indonesian", name)),

        // Spanish
        "id-es" => ("id", json_instr("Indonesian", "Español", name)),
        "es-id" => ("es", json_instr("Español", "Indonesian", name)),

        _ => ("id", json_instr("Indonesian", "English", name)),
    }
}


async fn handle_ws(mut socket: WebSocket, room: String, state: AppState) {
    let tx = match state.rooms.get(&room) {
        Some(t) => t.clone(),
        None => {
            let _ = socket
                .send(Message::Text("{\"error\":\"room not found\"}".into()))
                .await;
            return;
        }
    };
    info!("ws client connected for room {}", room);

    // Connect to OpenAI Realtime
    

   // ==== OpenAI Realtime handshake + logging detail ====
let url = format!("wss://api.openai.com/v1/realtime?model={}", state.model);
let key = generate_key();
let req = axum::http::Request::builder()
    .method("GET")
    .uri(&url)
    .header("Host", "api.openai.com")
    .header("Upgrade", "websocket")
    .header("Connection", "Upgrade")
    .header("Sec-WebSocket-Version", "13")
    .header("Sec-WebSocket-Key", key)
   .header("Sec-WebSocket-Protocol", "realtime")
    // <-- per subprotocol dicoba di sini
    .header("Authorization", format!("Bearer {}", state.api_key))
    .header("OpenAI-Beta", "realtime=v1")
    .body(())
    .unwrap();

info!("🔌 OpenAI connect → {}", url);
let hdrs = redact_headers(req.headers());
info!("🔎 Request headers: {:?}", hdrs);

// beri timeout agar terlihat kalau macet


const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
let res = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req)).await;

let (upstream, resp) = match res {
    Err(_) => {
        error!("⏱️ upstream connect timeout after {:?} to {}", CONNECT_TIMEOUT, url);
        let _ = socket
            .send(Message::Text(serde_json::json!({ "error": "upstream timeout" }).to_string()))
            .await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    Ok(Err(e)) => {
        error!("❌ upstream connect failed: {:?}", e);
        let msg = format!("upstream connect failed: {e}");
        let _ = socket
            .send(Message::Text(serde_json::json!({ "error": msg }).to_string()))
            .await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    Ok(Ok(p)) => p,
};

info!("✅ connected to OpenAI Realtime, status={}", resp.status());
let rh = redact_headers(resp.headers());
info!("🔎 Response headers: {:?}", rh);



    let (mut upstream_write, mut upstream_read) = upstream.split();

    // Shared flags
    let response_active = Arc::new(AtomicBool::new(false));
    let last_delta = Arc::new(Mutex::new(Instant::now()));
    let response_active_r = response_active.clone();
    let last_delta_r = last_delta.clone();

    // Reader: forward model deltas to SSE
    let tx_clone = tx.clone();
    let reader = tokio::spawn(async move {
        let mut current_buf = String::new();

        while let Some(msg) = upstream_read.next().await {
            match msg {
                Ok(tungstenite::Message::Text(txt)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("-");

                        match t {
                            "response.created" => {
                                info!("← response.created");
                                response_active_r.store(true, Ordering::SeqCst);
                            }
                            "response.output_text.delta" | "response.text.delta" => {
                                if let Some(delta) = v.get("delta").and_then(|x| x.as_str()) {
                                    *last_delta_r.lock().await = Instant::now();
                                    current_buf.push_str(delta);
                                    let _ = tx_clone.send(
                                        json!({"type":"partial","text": current_buf}).to_string()
                                    );
                                }
                            }
                            "response.delta" => {
                                if let Some(d) = v.get("delta") {
                                    if d.get("type").and_then(|x| x.as_str())
                                        == Some("output_text.delta")
                                    {
                                        if let Some(delta) = d.get("text").and_then(|x| x.as_str()) {
                                            *last_delta_r.lock().await = Instant::now();
                                            current_buf.push_str(delta);
                                            let _ = tx_clone.send(
                                                json!({"type":"partial","text": current_buf}).to_string()
                                            );
                                        }
                                    }
                                }
                            }
                            "response.output_text.done"
                            | "response.completed"
                            | "response.text.done"
                            | "response.done" => {
                                info!("← {}", t);
                                if !current_buf.is_empty() {
                                    let _ = tx_clone.send(
                                        json!({"type":"final","text": current_buf}).to_string()
                                    );
                                    current_buf.clear();
                                }
                                response_active_r.store(false, Ordering::SeqCst);
                            }
                            "error" => {
                                error!("← error: {}", txt);
                                let _ = tx_clone.send(json!({"type":"error","data": v}).to_string());
                                response_active_r.store(false, Ordering::SeqCst);
                            }
                            _ => { /* verbose silenced */ }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    error!("upstream read error: {}", e);
                    break;
                }
            }
        }
    });

    // Writer: receive from browser & forward to OpenAI
    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut inited = false;
    let mut audio_buffer_size: usize = 0;

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(t) => {
                if let Ok(v) = serde_json::from_str::<ClientMsg>(&t) {
                    match v {
                        ClientMsg::Init { name, pair } => {
                            let (src_lang, instr) = instructions_for(&pair, &name);

                            // IMPORTANT: disable server VAD to avoid conflicts with manual commit
                          let session_update = json!({
    "type": "session.update",
    "session": {
        "instructions": instr,
        "modalities": ["text"],
        "input_audio_transcription": {
            "model": "gpt-4o-mini-transcribe",
            "language": src_lang
        }
    }
});
                            log_upstream_json("session.update", &session_update);
                            let _ = upstream_write
                                .send(tungstenite::Message::Text(session_update.to_string()))
                                .await;

                            inited = true;
                            audio_buffer_size = 0;
                        }

                        ClientMsg::Commit => {
                            if !inited { continue; }
// Tunggu sebentar untuk memastikan append diproses
    tokio::time::sleep(Duration::from_millis(100)).await;
    
                            // Hitung durasi audio berdasarkan sample rate (default 24kHz)
    const SAMPLE_RATE: usize = 24000; // Hz
    const BYTES_PER_SAMPLE: usize = 2; // PCM16 = 2 bytes per sample
    const MIN_DURATION_MS: usize = 100; // minimal 100ms
    
    let min_samples = (SAMPLE_RATE * MIN_DURATION_MS) / 1000;
    let min_bytes = min_samples * BYTES_PER_SAMPLE;
    
    if audio_buffer_size < min_bytes {
        info!("skip commit: buffer has {}ms (need {}ms)", 
              (audio_buffer_size * 1000) / (SAMPLE_RATE * BYTES_PER_SAMPLE),
              MIN_DURATION_MS);
        continue;
    }
                            // If still streaming, cancel stale response (>800ms no delta)
                            if response_active.load(Ordering::SeqCst) {
                                let elapsed = last_delta.lock().await.elapsed();
                                if elapsed > Duration::from_millis(800) {
                                    let cancel = json!({"type":"response.cancel"});
                                    let _ = upstream_write
                                        .send(tungstenite::Message::Text(cancel.to_string()))
                                        .await;
                                    response_active.store(false, Ordering::SeqCst);
                                    info!("response.cancel (stale {:?})", elapsed);
                                } else {
                                    info!("skip response.create: still active (last delta {:?})", elapsed);
                                    continue;
                                }
                            }

                            let commit = json!({ "type": "input_audio_buffer.commit" });
                            log_upstream_json("input_audio_buffer.commit", &commit);
                            let _ = upstream_write
                                .send(tungstenite::Message::Text(commit.to_string()))
                                .await;

                           let create = json!({
  "type": "response.create",
  "response": {
    "modalities": ["text"],
    "conversation": "none",
    "temperature": 0.6
  }
});
                            log_upstream_json("response.create", &create);
                            let _ = upstream_write
                                .send(tungstenite::Message::Text(create.to_string()))
                                .await;

                            response_active.store(true, Ordering::SeqCst);
                            audio_buffer_size = 0; // reset after commit
                        }
                    }
                }
            }

            Message::Binary(bin) => {
                if !inited { continue; }

                audio_buffer_size += bin.len();
                 info!("Audio buffer: {} bytes ({}ms)", 
          audio_buffer_size,
          (audio_buffer_size * 1000) / (24000 * 2)); // 24kHz, PCM16


                let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&bin);
                let pkg = json!({ "type":"input_audio_buffer.append", "audio": audio_b64 });
                log_upstream_json("input_audio_buffer.append", &pkg);
                let _ = upstream_write
                    .send(tungstenite::Message::Text(pkg.to_string()))
                    .await;
            }

            Message::Close(_) => break,
            _ => {}
        }
    }

    let _ = ws_tx.send(Message::Close(None)).await;
    let _ = reader.await;
}

// Helper logging (redact base64 body size)
fn log_upstream_json(label: &str, v: &Value) {
    let mut red = v.clone();
    if let Some(obj) = red.as_object_mut() {
        if let Some(audio) = obj.get_mut("audio") {
            if let Some(s) = audio.as_str() {
                *audio = Value::String(format!("<base64:{} chars>", s.len()));
            }
        }
    }
    let pretty = serde_json::to_string_pretty(&red).unwrap_or_else(|_| red.to_string());
    info!("→ {} {}", label, pretty);
}
