use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use reqwest::{Client, Url};
use varnish::vcl::{Backend, BodyState, StrOrBytes, VclBackend, VclResponse};
use varnish::vcl::{
    Buffer, Ctx, Event, LogTag, Probe, Request as ProbeRequest, VclError, VclResult, log,
};

use super::vcl_client::BgThread;

/// State shared between the backend and its probe task. The task owns an
/// `Arc` clone, so it never borrows from `ProbeState` and stays sound even if
/// it is still polling when the backend is freed (`abort()` is asynchronous).
struct ProbeShared {
    history: AtomicU64,
    avg: Mutex<f64>,
    health_changed: Mutex<SystemTime>,
}

pub struct ProbeState {
    spec: Probe,
    shared: Arc<ProbeShared>,
    url: Url,
    join_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    accept_invalid_certs: bool,
    accept_invalid_hostnames: bool,
}
#[allow(non_camel_case_types)]
pub struct client {
    pub name: String,
    pub be: Backend<VCLBackend, BackendResp>,
}

pub struct VCLBackend {
    pub name: String,
    pub bgt: *const BgThread,
    pub client: Client,
    pub blocking_client: reqwest::blocking::Client,
    pub probe_state: Option<ProbeState>,
    pub https: bool,
    pub base_url: Option<String>,
}

// silly helper until varnish-rs provides something more ergonomic
fn sob_helper<'a>(sob: &'a StrOrBytes) -> &'a str {
    match sob {
        StrOrBytes::Bytes(_) => panic!("{sob:?} isn't a string"),
        StrOrBytes::Utf8(s) => s,
    }
}

#[allow(clippy::extra_unused_lifetimes)]
impl<'a> VclBackend<BackendResp> for VCLBackend {
    fn get_response(&self, ctx: &mut Ctx<'_>) -> VclResult<Option<BackendResp>> {
        if !self.probe(ctx).0 {
            return Err("unhealthy".into());
        }

        let bereq = ctx.http_bereq.as_ref().unwrap();

        let sob = bereq.url().unwrap();
        let bereq_url = sob_helper(&sob);

        let url = if let Some(base_url) = &self.base_url {
            // if the client has a base_url, prepend it to bereq.url
            format!("{base_url}{bereq_url}")
        } else if bereq_url.starts_with('/') {
            // otherwise, if bereq.url looks like a path, try to find a host to build a full URL
            if let Some(host) = bereq.header("host") {
                let host_str = sob_helper(&host);
                format!(
                    "{}://{}{}",
                    if self.https { "https" } else { "http" },
                    host_str,
                    bereq_url
                )
            } else {
                return Err("no host found (reqwest.client doesn't have a base_url, bereq.url doesn't specify a host and bereq.http.host is unset)".into());
            }
        } else {
            // else use bereq.url as-is
            bereq_url.to_string()
        };

        let method = sob_helper(&bereq.method().unwrap()).to_string();
        let headers: Vec<(String, Vec<u8>)> = bereq
            .into_iter()
            .map(|(k, v)| (k.into(), v.as_ref().to_owned()))
            .collect();

        let mut body: Option<Vec<u8>> = None;

        if ctx.req_body_state()? != BodyState::None {
            let mut body_vec = Vec::new();
            ctx.req_body(&mut body_vec)?;
            body = Some(body_vec);
        }

        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| <String as Into<VclError>>::into(e.to_string()))?;
        let mut rb = self.blocking_client.request(method, url);
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        if let Some(body) = body {
            rb = rb.body(body);
        }
        let resp = rb
            .send()
            .map_err(|e| <String as Into<VclError>>::into(e.to_string()))?;

        let beresp = ctx.http_beresp.as_mut().unwrap();
        beresp.set_status(resp.status().as_u16());
        beresp.set_proto("HTTP/1.1")?;
        for (k, v) in resp.headers() {
            beresp.set_header(
                k.as_str(),
                v.to_str()
                    .map_err(|e| <String as Into<VclError>>::into(e.to_string()))?,
            )?;
        }
        let content_length = resp
            .content_length()
            .map(|s| {
                usize::try_from(s).map_err(|e| {
                    <String as Into<VclError>>::into(format!("invalid content-length ({s}): {e}"))
                })
            })
            .transpose()?;
        Ok(Some(BackendResp {
            resp,
            content_length,
        }))
    }

    fn probe(&self, _ctx: &mut Ctx<'_>) -> (bool, SystemTime) {
        let Some(ref probe_state) = self.probe_state else {
            return (true, SystemTime::UNIX_EPOCH);
        };

        assert!(probe_state.spec.window <= 64);

        let bitmap = probe_state.shared.history.load(Ordering::Relaxed);
        (
            is_healthy(bitmap, probe_state.spec.window, probe_state.spec.threshold),
            *probe_state
                .shared
                .health_changed
                .lock()
                .expect("health_changed mutex poisoned"),
        )
    }

    fn event(&self, event: Event) {
        // nothing to do
        let Some(ref probe_state) = self.probe_state else {
            return;
        };

        // SAFETY: the `BgThread` lives in the VCL's `shared_per_vcl` priv,
        // which outlives every backend of that VCL.
        let bgt = unsafe { &*self.bgt };
        // Varnish alternates Warm and Cold events for a backend, so Warm never
        // finds a running task and aborting an old handle there is purely
        // defensive.
        match event {
            // start the probing loop
            Event::Warm => {
                let handle = spawn_probe(bgt.rt.handle(), probe_state, self.name.clone(), |msg| {
                    log(LogTag::BackendHealth, msg);
                });
                let old = probe_state
                    .join_handle
                    .lock()
                    .expect("join_handle mutex poisoned")
                    .replace(handle);
                if let Some(old) = old {
                    old.abort();
                }
            }
            Event::Cold => {
                let handle = probe_state
                    .join_handle
                    .lock()
                    .expect("join_handle mutex poisoned")
                    .take();
                if let Some(handle) = handle {
                    handle.abort();
                }
            }
            _ => {}
        }
    }

    fn report(&self, _ctx: &mut Ctx<'_>, vsb: &mut Buffer<'_>) {
        let Some(ProbeState {
            shared,
            spec: Probe {
                window, threshold, ..
            },
            ..
        }) = self.probe_state.as_ref()
        else {
            return;
        };
        let bitmap = shared.history.load(Ordering::Relaxed);
        vsb.write(&format!(
            "{}/{}\t{}",
            good_probes(bitmap, *window),
            window,
            if is_healthy(bitmap, *window, *threshold) {
                "healthy"
            } else {
                "sick"
            }
        ))
        .expect("vsb buffer full");
    }

    fn report_details(&self, ctx: &mut Ctx<'_>, vsb: &mut Buffer<'_>) {
        let Some(ProbeState {
            shared,
            spec: Probe {
                window, threshold, ..
            },
            ..
        }) = self.probe_state.as_ref()
        else {
            let state = if self.probe(ctx).0 { "healthy" } else { "sick" };
            vsb.write(&"0/0\t").expect("vsb buffer full");
            vsb.write(&state).expect("vsb buffer full");
            return;
        };
        let bitmap = shared.history.load(Ordering::Relaxed);
        let window = *window;
        let threshold = *threshold;
        let mut s = format!(
            "
 Current states  good: {:2} threshold: {:2} window: {:2}
  Average response time of good probes: {:.06}
  Oldest ================================================== Newest
  ",
            good_probes(bitmap, window),
            threshold,
            window,
            shared.avg.lock().expect("avg mutex poisoned")
        );
        for i in 0..64 {
            s += if bitmap.wrapping_shr(63 - i) & 1 == 1 {
                "H"
            } else {
                "-"
            };
        }
        vsb.write(&s).expect("vsb buffer full");
    }

    fn report_json(&self, _ctx: &mut Ctx<'_>, vsb: &mut Buffer<'_>) {
        let Some(ProbeState {
            shared,
            spec: Probe {
                window, threshold, ..
            },
            ..
        }) = self.probe_state.as_ref()
        else {
            vsb.write(&"[]").expect("vsb buffer full");
            return;
        };
        let bitmap = shared.history.load(Ordering::Relaxed);
        vsb.write(&format!(
            "[{}, {}, \"{}\"]",
            good_probes(bitmap, *window),
            window,
            if is_healthy(bitmap, *window, *threshold) {
                "healthy"
            } else {
                "sick"
            }
        ))
        .expect("vsb buffer full");
    }

    fn report_details_json(&self, ctx: &mut Ctx<'_>, vsb: &mut Buffer<'_>) {
        let Some(ref probe_state) = self.probe_state else {
            let state = if self.probe(ctx).0 { "healthy" } else { "sick" };
            vsb.write(&"[0, 0, \"").expect("vsb buffer full");
            vsb.write(&state).expect("vsb buffer full");
            vsb.write(&"\"],").expect("vsb buffer full");
            return;
        };
        // TODO: talk to upstream, we shouldn't have to add the comma
        let msg = serde_json::to_string(&probe_state.spec)
            .expect("probe spec serialization failed")
            + ",\n";
        vsb.write(&msg).expect("vsb buffer full");
    }
}

pub struct BackendResp {
    pub resp: reqwest::blocking::Response,
    pub content_length: Option<usize>,
}

impl VclResponse for BackendResp {
    fn read(&mut self, buf: &mut [u8]) -> VclResult<usize> {
        use std::io::Read;
        self.resp
            .read(buf)
            .map_err(|e| VclError::new(e.to_string()))
    }

    fn len(&self) -> Option<usize> {
        self.content_length
    }
}

fn good_probes(bitmap: u64, window: u32) -> u32 {
    bitmap.wrapping_shl(64_u32 - window).count_ones()
}

fn is_healthy(bitmap: u64, window: u32, threshold: u32) -> bool {
    good_probes(bitmap, window) >= threshold
}

fn update_health(
    mut bitmap: u64,
    threshold: u32,
    window: u32,
    probe_ok: bool,
) -> (u64, bool, bool) {
    let old_health = is_healthy(bitmap, window, threshold);
    let new_bit = u64::from(probe_ok);
    bitmap = bitmap.wrapping_shl(1) | new_bit;
    let new_health = is_healthy(bitmap, window, threshold);
    (bitmap, new_health, new_health == old_health)
}

/// Spawn the probing loop on `rt`. The task only owns copies of what it needs
/// (notably an `Arc` clone of `probe_state.shared`) and borrows nothing from
/// `probe_state`, so nothing it touches can be freed under it. `log_health`
/// receives each `BackendHealth` line (a parameter so unit tests can run
/// without varnishd's `VSL`).
fn spawn_probe<F>(
    rt: &tokio::runtime::Handle,
    probe_state: &ProbeState,
    name: String,
    log_health: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(String) + Send + 'static,
{
    let spec = probe_state.spec.clone();
    let url = probe_state.url.clone();
    let shared = Arc::clone(&probe_state.shared);
    let accept_invalid_certs = probe_state.accept_invalid_certs;
    let accept_invalid_hostnames = probe_state.accept_invalid_hostnames;
    rt.spawn(async move {
        let history = &shared.history;
        let avg = &shared.avg;
        let mut h = 0_u64;
        for i in 0..std::cmp::min(spec.initial, 64) {
            h |= 1 << i;
        }
        history.store(h, Ordering::Relaxed);
        let mut avg_rate = 0_f64;
        loop {
            let msg;
            let mut time = 0_f64;
            let new_bit = match reqwest::ClientBuilder::new()
                .timeout(spec.timeout)
                .danger_accept_invalid_certs(accept_invalid_certs)
                .danger_accept_invalid_hostnames(accept_invalid_hostnames)
                .build()
                .map(|req| req.get(url.clone()).send())
            {
                Err(e) => {
                    msg = e.to_string();
                    false
                }
                Ok(resp) => {
                    let start = Instant::now();
                    match resp.await {
                        Err(e) => {
                            msg = format!("Error: {e}");
                            false
                        }
                        Ok(resp) if u32::from(resp.status().as_u16()) == spec.exp_status => {
                            msg = format!("Success: {}", resp.status().as_u16());
                            if avg_rate < 4.0 {
                                avg_rate += 1.0;
                            }
                            time = start.elapsed().as_secs_f64();
                            let mut avg = avg.lock().unwrap();
                            *avg += (time - *avg) / avg_rate;
                            true
                        }
                        Ok(resp) => {
                            msg = format!(
                                "Error: expected {} status, got {}",
                                spec.exp_status,
                                resp.status().as_u16()
                            );
                            false
                        }
                    }
                }
            };
            let bitmap = history.load(Ordering::Relaxed);
            let (bitmap, healthy, changed) =
                update_health(bitmap, spec.threshold, spec.window, new_bit);
            log_health(format!(
                "{} {} {} {} {} {} {} {} {} {}",
                name,
                if changed { "Went" } else { "Still" },
                if healthy { "healthy" } else { "sick" },
                "UNIMPLEMENTED",
                good_probes(bitmap, spec.window),
                spec.threshold,
                spec.window,
                time,
                *avg.lock().unwrap(),
                msg
            ));
            history.store(bitmap, Ordering::Relaxed);
            tokio::time::sleep(spec.interval).await;
        }
    })
}

pub fn build_probe_state(
    mut probe: Probe,
    base_url: Option<&str>,
    accept_invalid_certs: bool,
    accept_invalid_hostnames: bool,
) -> Result<ProbeState, VclError> {
    // sanitize probe (see vbp_set_defaults in Varnish Cache)
    if probe.timeout.is_zero() {
        probe.timeout = Duration::from_secs(2);
    }
    if probe.interval.is_zero() {
        probe.interval = Duration::from_secs(5);
    }
    if probe.window == 0 {
        probe.window = 8;
    }
    if probe.threshold == 0 {
        probe.threshold = 3;
    }
    if probe.exp_status == 0 {
        probe.exp_status = 200;
    }
    if probe.initial == 0 {
        probe.initial = probe.threshold - 1;
    }
    probe.initial = std::cmp::min(probe.initial, probe.threshold);
    let spec_url = match probe.request {
        ProbeRequest::Url(ref u) => u,
        ProbeRequest::Text(_) => {
            return Err(VclError::new("can't use a probe without .url".to_string()));
        }
    };
    let url = if let Some(base_url) = base_url {
        let full_url = format!("{base_url}{spec_url}");
        Url::parse(&full_url)
            .map_err(|e| VclError::new(format!("problem with probe endpoint {full_url} ({e})")))?
    } else if spec_url.starts_with('/') {
        return Err(VclError::new(
            "client has no .base_url, and the probe doesn't have a fully-qualified URL as .url"
                .to_string(),
        ));
    } else {
        Url::parse(spec_url)
            .map_err(|e| VclError::new(format!("probe endpoint {spec_url} ({e})")))?
    };
    Ok(ProbeState {
        spec: probe,
        shared: Arc::new(ProbeShared {
            history: AtomicU64::new(0),
            avg: Mutex::new(0_f64),
            health_changed: Mutex::new(SystemTime::now()),
        }),
        join_handle: Mutex::new(None),
        url,
        accept_invalid_certs,
        accept_invalid_hostnames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Guard, not a fail-before test: the old code could not even express this
    // (the task borrowed `ProbeState` fields for 'static). It checks that the
    // probe task holds no state beyond its own `Arc` clone, which it drops once
    // cancelled, so freeing the backend after Cold can't leave the task with a
    // dangling reference.
    #[test]
    fn aborted_probe_task_releases_shared_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // closed local port, so each poll fails fast
        let probe = Probe {
            request: ProbeRequest::Url("http://127.0.0.1:1/probe".to_string()),
            timeout: Duration::from_secs(1),
            interval: Duration::from_millis(10),
            exp_status: 200,
            window: 8,
            threshold: 3,
            initial: 2,
        };
        let state = build_probe_state(probe, None, false, false).unwrap();
        let handle = spawn_probe(rt.handle(), &state, "test".to_string(), |_| {});
        assert_eq!(Arc::strong_count(&state.shared), 2);
        // let the task run a few iterations
        std::thread::sleep(Duration::from_millis(50));
        handle.abort();
        let err = rt.block_on(handle).unwrap_err();
        assert!(err.is_cancelled());
        assert_eq!(Arc::strong_count(&state.shared), 1);
    }
}
