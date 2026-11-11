//! What the program needs from a browser: requests through `fetch`, and a way to wake the
//! window when one of them is answered. A page has one thread, so nothing here may block:
//! requests are started, and their answers picked up on a later frame.

use crate::fetcher::{Answer, BodyError};
use crate::wtime::{Duration, Instant};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// How long a request may take before it counts as timed out.
const TIMEOUT_MS: u32 = 30_000;
/// An answer nobody came back for is dropped after this long.
const UNCLAIMED: Duration = Duration::from_secs(60);

thread_local! {
    static WAKER: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Sets what is called whenever an answer comes in, so the window looks at it.
pub fn set_waker(f: impl Fn() + 'static) {
    WAKER.with(|w| *w.borrow_mut() = Some(Box::new(f)));
}

pub fn wake() {
    WAKER.with(|w| {
        if let Some(f) = &*w.borrow() {
            f();
        }
    });
}

#[derive(Default)]
struct Inner {
    /// Requests under way, with their host.
    pending: HashMap<String, String>,
    /// Answers not yet collected, and when they came.
    done: HashMap<String, (Answer, Instant)>,
    per_host: HashMap<String, usize>,
}

/// Requests made from the page, by address.
#[derive(Default, Clone)]
pub struct Downloads {
    inner: Arc<Mutex<Inner>>,
}

impl Downloads {
    pub fn take(&self, url: &str) -> Option<Answer> {
        self.inner.lock().unwrap().done.remove(url).map(|d| d.0)
    }

    /// The answer, left in place: a job that is run again until all its answers are in needs
    /// those it already had, which it may not have kept.
    pub fn peek(&self, url: &str) -> Option<Answer> {
        self.inner
            .lock()
            .unwrap()
            .done
            .get(url)
            .map(|d| d.0.clone())
    }

    pub fn is_pending(&self, url: &str) -> bool {
        self.inner.lock().unwrap().pending.contains_key(url)
    }

    /// Whether waiting for this address is over: its answer is in, or it was never asked.
    pub fn is_settled(&self, url: &str) -> bool {
        !self.is_pending(url)
    }

    pub fn in_flight(&self, host: &str) -> usize {
        self.inner
            .lock()
            .unwrap()
            .per_host
            .get(host)
            .copied()
            .unwrap_or(0)
    }

    pub fn start(&self, url: &str, host: &str) {
        {
            let mut i = self.inner.lock().unwrap();
            let now = Instant::now();
            i.done.retain(|_, d| now.duration_since(d.1) < UNCLAIMED);
            i.pending.insert(url.to_string(), host.to_string());
            *i.per_host.entry(host.to_string()).or_default() += 1;
        }
        let (me, url) = (self.clone(), url.to_string());
        wasm_bindgen_futures::spawn_local(async move {
            let answer = get(&url).await;
            {
                let mut i = me.inner.lock().unwrap();
                if let Some(host) = i.pending.remove(&url) {
                    if let Some(n) = i.per_host.get_mut(&host) {
                        *n = n.saturating_sub(1);
                    }
                }
                i.done.insert(url, (answer, Instant::now()));
            }
            wake();
        });
    }
}

/// One GET request, as the browser lets a page make it: across sites only where the server
/// allows that, and with the browser's own name for itself.
pub async fn get(url: &str) -> Answer {
    match try_get(url).await {
        Ok(a) => a,
        Err(e) => {
            let name = js_sys::Reflect::get(&e, &"name".into())
                .ok()
                .and_then(|n| n.as_string());
            if name.as_deref() == Some("TimeoutError") {
                Answer::TimedOut
            } else {
                // A refused cross-site request looks just like an unreachable server.
                let text = js_sys::Reflect::get(&e, &"message".into())
                    .ok()
                    .and_then(|n| n.as_string());
                Answer::Unreachable(text.unwrap_or_else(|| "request failed".into()))
            }
        }
    }
}

async fn try_get(url: &str) -> Result<Answer, wasm_bindgen::JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(TIMEOUT_MS)));
    let request = web_sys::Request::new_with_str_and_init(url, &init)?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_request(&request))
        .await?
        .dyn_into()?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = if (200..300).contains(&status) {
        match response.array_buffer() {
            Ok(promise) => match JsFuture::from(promise).await {
                Ok(buffer) => Ok(js_sys::Uint8Array::new(&buffer).to_vec()),
                Err(e) => {
                    let name = js_sys::Reflect::get(&e, &"name".into())
                        .ok()
                        .and_then(|n| n.as_string());
                    Err(if name.as_deref() == Some("TimeoutError") {
                        BodyError::TimedOut
                    } else {
                        BodyError::Broken
                    })
                }
            },
            Err(_) => Err(BodyError::Broken),
        }
    } else {
        Ok(Vec::new())
    };
    Ok(Answer::Reply {
        status,
        retry_after,
        body,
    })
}

/// Writes to the browser console, where `eprintln!` would go nowhere.
pub fn log(text: &str) {
    web_sys::console::log_1(&text.into());
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// The page's drawing area: the element with id `orbis`, or a new one.
pub fn canvas() -> Option<web_sys::HtmlCanvasElement> {
    document()?.get_element_by_id("orbis")?.dyn_into().ok()
}

/// The page address's options as command-line words: `?lat=46.5&tilt` is `--lat 46.5 --tilt`.
/// The part after `#` counts too, so links can carry options without reloading the page.
pub fn address_args() -> Vec<String> {
    let Some(location) = web_sys::window().map(|w| w.location()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in [
        location.search().unwrap_or_default(),
        location.hash().unwrap_or_default(),
    ] {
        let Ok(params) =
            web_sys::UrlSearchParams::new_with_str(part.trim_start_matches(['?', '#']))
        else {
            continue;
        };
        let entries = js_sys::try_iter(&params.entries()).ok().flatten();
        for entry in entries.into_iter().flatten().flatten() {
            let pair = js_sys::Array::from(&entry);
            let (Some(key), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string())
            else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            out.push(format!("--{key}"));
            if !value.is_empty() {
                out.push(value);
            }
        }
    }
    out
}

/// Tells the page the program is running, so it can take down its loading notice.
pub fn started() {
    call_page("orbisStarted", "");
}

/// Shows why the program could not start, on the page rather than only in the console.
pub fn fail(why: &str) {
    log(why);
    call_page("orbisFailed", why);
}

fn call_page(name: &str, text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Ok(f) = js_sys::Reflect::get(&window, &name.into()) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&wasm_bindgen::JsValue::NULL, &text.into());
        }
    }
}

thread_local! {
    static BUDGET_END: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Gives background work, which runs between frames here, this much time from now.
pub fn set_budget(time: Duration) {
    BUDGET_END.with(|b| b.set(Some(Instant::now() + time)));
}

/// Whether background work may go on before the next frame is due.
pub fn has_budget() -> bool {
    BUDGET_END.with(|b| b.get().is_some_and(|end| Instant::now() < end))
}

/// Opens an address in a new tab.
pub fn open(url: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("No window to open it from.")?;
    window
        .open_with_url_and_target(url, "_blank")
        .map(|_| ())
        .map_err(|_| "The browser did not open it.".to_string())
}

thread_local! {
    static WAITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Notes that the job now running is waiting for a download, so its answer does not count.
pub fn note_waiting() {
    WAITING.with(|w| w.set(true));
}

pub fn clear_waiting() {
    WAITING.with(|w| w.set(false));
}

pub fn take_waiting() -> bool {
    WAITING.with(|w| w.replace(false))
}

/// Gives the canvas as many pixels as the screen has under it, and returns that size. The page
/// lays the canvas out in CSS pixels; a sharp picture needs `devicePixelRatio` times as many.
pub fn fit_canvas() -> Option<(u32, u32)> {
    let window = web_sys::window()?;
    let canvas = canvas()?;
    let ratio = window.device_pixel_ratio();
    let width = (canvas.client_width() as f64 * ratio).round() as u32;
    let height = (canvas.client_height() as f64 * ratio).round() as u32;
    if width == 0 || height == 0 {
        return None;
    }
    if canvas.width() != width || canvas.height() != height {
        canvas.set_width(width);
        canvas.set_height(height);
    }
    Some((width, height))
}
