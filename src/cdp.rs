//! Minimal Chrome DevTools Protocol client over --remote-debugging-pipe (fd 3 in, fd 4 out, NUL-framed JSON).
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::pipe;
use tokio::sync::{mpsc, oneshot};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Clone)]
pub struct Cdp {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    pending: Pending,
    next: Arc<AtomicU64>,
}

pub struct Event {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

impl Cdp {
    /// Takes ownership of our ends of the two pipes. Returns client + event stream.
    /// The event stream ends (None) when the browser closes the pipe.
    pub fn start(writer: pipe::Sender, reader: pipe::Receiver) -> (Cdp, mpsc::UnboundedReceiver<Event>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (etx, erx) = mpsc::unbounded_channel::<Event>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let mut w = writer;
        tokio::spawn(async move {
            while let Some(mut m) = rx.recv().await {
                m.push(0);
                if w.write_all(&m).await.is_err() {
                    break;
                }
            }
        });
        let p2 = pending.clone();
        tokio::spawn(async move {
            let mut r = BufReader::with_capacity(1 << 20, reader);
            let mut buf = Vec::with_capacity(1 << 20);
            loop {
                buf.clear();
                match r.read_until(0, &mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if buf.last() == Some(&0) {
                    buf.pop();
                }
                let v: Value = match serde_json::from_slice(&buf) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(id) = v.get("id").and_then(|x| x.as_u64()) {
                    if let Some(s) = p2.lock().unwrap().remove(&id) {
                        let r = if let Some(e) = v.get("error") {
                            Err(e.to_string())
                        } else {
                            Ok(v.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = s.send(r);
                    }
                } else if let Some(m) = v.get("method").and_then(|x| x.as_str()) {
                    let _ = etx.send(Event {
                        method: m.to_string(),
                        params: v.get("params").cloned().unwrap_or(Value::Null),
                        session: v.get("sessionId").and_then(|x| x.as_str()).map(String::from),
                    });
                }
            }
            // fail all pending calls
            for (_, s) in p2.lock().unwrap().drain() {
                let _ = s.send(Err("pipe closed".into()));
            }
        });
        (
            Cdp {
                tx,
                pending,
                next: Arc::new(AtomicU64::new(1)),
            },
            erx,
        )
    }

    pub async fn call(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout_s: u64,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (s, r) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, s);
        if self.tx.send(serde_json::to_vec(&msg).unwrap()).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("pipe closed".into());
        }
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), r).await {
            Ok(Ok(res)) => res.map_err(|e| format!("{method}: {e}")),
            Ok(Err(_)) => Err(format!("{method}: dropped")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("{method}: timeout after {timeout_s}s"))
            }
        }
    }
}
