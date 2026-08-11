use std::boxed::Box;

use anyhow::Error;
use bytes::Bytes;
use reqwest::Client;
use reqwest::header::HeaderMap;
use tokio::sync::mpsc::{Receiver, Sender, UnboundedSender};
use varnish::vcl::{VclError, VclResult};

use super::backend::client;

macro_rules! send {
    ($tx:ident, $payload:expr) => {
        if $tx.send($payload).await.is_err() {
            return;
        }
    };
}

#[derive(Debug)]
pub struct Entry {
    pub client_name: String,
    pub req_name: String,
    pub transaction: VclTransaction,
}

// try to keep the object on stack as small as possible, we'll flesh it out into a reqwest::Request
// once in the Background thread
#[derive(Debug)]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Option<reqwest::Body>,
    pub client: Client,
}

// calling reqwest::Response::body() consumes the object, so we keep a copy of the interesting bits
// in this struct
#[derive(Debug)]
pub struct Response {
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub status: i64,
}

#[derive(Debug)]
pub enum VclTransaction {
    Transition,
    Req(Request),
    Sent(Receiver<Result<Response, Error>>),
    Resp(Result<Response, VclError>),
}

impl VclTransaction {
    fn unwrap_resp(&self) -> Result<&Response, VclError> {
        match self {
            VclTransaction::Resp(Ok(rsp)) => Ok(rsp),
            VclTransaction::Resp(Err(e)) => Err(VclError::new(e.to_string())),
            _ => panic!("wrong VclTransaction type"),
        }
    }
    fn into_req(self) -> Request {
        match self {
            VclTransaction::Req(rq) => rq,
            _ => panic!("wrong VclTransaction type"),
        }
    }
}

pub struct BgThread {
    pub rt: tokio::runtime::Runtime,
    pub sender: UnboundedSender<(Request, Sender<Result<Response, Error>>)>,
}

impl BgThread {
    fn spawn_req(&self, req: Request) -> Receiver<Result<Response, Error>> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        self.sender.send((req, tx)).expect(
            "BgThread's receiver loop is gone, but it must outlive every client backed by this VCL",
        );
        rx
    }
}

pub async fn process_req(req: Request, tx: Sender<Result<Response, Error>>) {
    let method = match reqwest::Method::from_bytes(req.method.as_bytes()) {
        Ok(m) => m,
        Err(e) => {
            send!(tx, Err(e.into()));
            return;
        }
    };
    let mut rreq = req.client.request(method, req.url);
    for (k, v) in req.headers {
        rreq = rreq.header(k, v);
    }
    if let Some(body) = req.body {
        rreq = rreq.body(body);
    }
    let resp = match rreq.send().await {
        Err(e) => {
            send!(tx, Err(e.into()));
            return;
        }
        Ok(resp) => resp,
    };
    let status = i64::from(resp.status().as_u16());
    let headers = resp.headers().clone();
    let body = match resp.bytes().await {
        Err(e) => {
            send!(tx, Err(e.into()));
            return;
        }
        Ok(b) => b,
    };
    send!(
        tx,
        Ok(Response {
            status,
            headers,
            body: Some(body),
        })
    );
}

impl client {
    pub fn vcl_send(bgt: &BgThread, t: &mut VclTransaction) {
        let old_t = std::mem::replace(t, VclTransaction::Transition);
        *t = VclTransaction::Sent(bgt.spawn_req(old_t.into_req()));
    }

    pub fn wait_on(bgt: &BgThread, t: &mut VclTransaction) {
        match t {
            VclTransaction::Req(_) => {
                Self::vcl_send(bgt, t);
                Self::wait_on(bgt, t);
            }
            VclTransaction::Sent(rx) => {
                *t = match rx
                    .blocking_recv()
                    .expect("BgThread dropped the response sender without replying")
                {
                    Ok(resp) => VclTransaction::Resp(Ok(resp)),
                    Err(e) => VclTransaction::Resp(Err(format!("{e}: {}", e.root_cause()).into())),
                };
            }
            VclTransaction::Resp(_) => (),
            VclTransaction::Transition => panic!("impossible"),
        }
    }

    pub fn get_transaction<'a>(
        &self,
        vp_task: &'a mut Option<Box<Vec<Entry>>>,
        name: &'a str,
    ) -> VclResult<&'a mut VclTransaction> {
        vp_task
            .as_mut()
            .ok_or_else(|| {
                <String as Into<VclError>>::into(format!(
                    "reqwest.get_transaction(): unknown request ({name})"
                ))
            })?
            .iter_mut()
            .find(|e| name == e.req_name && self.name == e.client_name)
            .map(|e| &mut e.transaction)
            .ok_or_else(|| {
                <String as Into<VclError>>::into(format!(
                    "reqwest.get_transaction(): unknown request ({name})"
                ))
            })
    }

    pub fn get_req<'a>(
        &self,
        vp_task: &'a mut Option<Box<Vec<Entry>>>,
        name: &'a str,
    ) -> VclResult<&'a mut Request> {
        match self.get_transaction(vp_task, name)? {
            VclTransaction::Req(req) => Ok(req),
            _ => Err(format!("reqwest.get_req(): request ({name}) already sent").into()),
        }
    }

    // we have a stacked Result here because the first one will fail at the
    // vcl level, while the core one is salvageable
    pub fn get_resp<'a>(
        &self,
        vp_vcl: Option<&BgThread>,
        vp_task: &'a mut Option<Box<Vec<Entry>>>,
        name: &'a str,
    ) -> VclResult<Result<&'a Response, VclError>> {
        let t = self.get_transaction(vp_task, name)?;
        Self::wait_on(
            vp_vcl
                .as_ref()
                .expect("BgThread priv should be initialized for the lifetime of the VCL"),
            t,
        );
        Ok(t.unwrap_resp())
    }
}
