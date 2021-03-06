// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod allow;
mod auth_reqs;
mod authed_apps;
mod authorise;
mod create_acc;
mod deny;
mod log_in;
mod log_out;
mod revoke;
mod status;
mod subscribe;
mod unsubscribe;

use crate::errors::Error;
use crate::shared::{
    lock_safe_authenticator, remove_auth_req_from_list, SharedAuthReqsHandle,
    SharedNotifEndpointsHandle, SharedSafeAuthenticatorHandle,
};
use futures::{Async, Future, Poll, Stream};
use std::str;
use tokio::sync::mpsc;

type FutureItemType = Box<[u8]>;
type FutureErrorType = Error;

// TODO: implement JSON-RPC rather.
// Temporarily prefix message with "[AUTHD_ERROR]" to signal error to the caller,
// once we have JSON-RPC we can adhere to its format for errors.
fn err_response(str: String) -> FutureItemType {
    format!("[AUTHD_ERROR]:SAFE Authenticator: {}", str)
        .into_bytes()
        .into()
}

fn successful_response(str: String) -> FutureItemType {
    str.into_bytes().into()
}

pub fn process_request(
    safe_auth_handle: SharedSafeAuthenticatorHandle,
    auth_reqs_handle: SharedAuthReqsHandle,
    notif_endpoints_handle: SharedNotifEndpointsHandle,
    req: &[u8],
) -> ProcessRequest {
    ProcessRequest::HandleRequest {
        safe_auth_handle,
        auth_reqs_handle,
        notif_endpoints_handle,
        req: req.to_vec(),
    }
}

pub enum ProcessRequest {
    HandleRequest {
        safe_auth_handle: SharedSafeAuthenticatorHandle,
        auth_reqs_handle: SharedAuthReqsHandle,
        notif_endpoints_handle: SharedNotifEndpointsHandle,
        req: Vec<u8>,
    },
    ProcessingResponse {
        safe_auth_handle: SharedSafeAuthenticatorHandle,
        auth_reqs_handle: SharedAuthReqsHandle,
        rx: mpsc::Receiver<bool>,
        req_id: u32,
        auth_req_str: String,
    },
}

impl Future for ProcessRequest {
    type Item = FutureItemType;
    type Error = FutureErrorType;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        use self::ProcessRequest::*;

        loop {
            match self {
                HandleRequest {
                    safe_auth_handle,
                    auth_reqs_handle,
                    notif_endpoints_handle,
                    req,
                } => {
                    if req.len() < 4 || &req[0..4] != b"GET " {
                        return Ok(Async::Ready(err_response("Missing GET".to_string())));
                    }
                    if req[4..].len() < 2 || &req[req.len() - 2..] != b"\r\n" {
                        return Ok(Async::Ready(err_response("Missing \\r\\n".to_string())));
                    }
                    let req = &req[4..req.len() - 2];
                    let end = req
                        .iter()
                        .position(|&c| c == b' ')
                        .unwrap_or_else(|| req.len());
                    let path = match str::from_utf8(&req[..end]) {
                        Ok(path) => path,
                        Err(err) => {
                            return Ok(Async::Ready(err_response(format!(
                                "Path is malformed UTF-8: {}",
                                err
                            ))))
                        }
                    };
                    let req_args: Vec<&str> = path.split('/').collect();

                    match process_authenticator_req(
                        req_args,
                        safe_auth_handle.clone(),
                        auth_reqs_handle.clone(),
                        notif_endpoints_handle.clone(),
                    ) {
                        Ok(AuthdResponse::Ready(response)) => return Ok(Async::Ready(response)),
                        Err(err) => return Err(err),
                        Ok(AuthdResponse::NotReady(processing_resp)) => *self = processing_resp,
                    };
                }
                ProcessingResponse {
                    safe_auth_handle,
                    auth_reqs_handle,
                    rx,
                    req_id,
                    auth_req_str,
                } => {
                    match rx.poll() {
                        Ok(Async::Ready(Some(is_allowed))) => {
                            if is_allowed {
                                println!("Let's request the authenticator lib to authorise the request '{}'...", req_id);
                                match lock_safe_authenticator(
                                    safe_auth_handle.clone(),
                                    |safe_authenticator| match safe_authenticator
                                        .authorise_app(auth_req_str)
                                    {
                                        Ok(resp) => {
                                            println!("Authorisation request ({}) was allowed and response sent back to the application", req_id);
                                            Ok(resp)
                                        }
                                        Err(err) => {
                                            println!("Failed to authorise application: {}", err);
                                            Ok(err.to_string())
                                        }
                                    },
                                ) {
                                    Ok(resp) => return Ok(Async::Ready(successful_response(resp))),
                                    Err(err) => return Ok(Async::Ready(err_response(err))),
                                }
                            } else {
                                let msg = format!("Authorisation request ({}) was denied", req_id);
                                println!("{}", msg);
                                return Ok(Async::Ready(err_response(msg)));
                            }
                        }
                        Ok(Async::NotReady) => {
                            return Ok(Async::NotReady);
                        }
                        Ok(Async::Ready(None)) | Err(_) => {
                            // We didn't get a response in a timely manner, we cannot allow the list
                            // to grow infinitelly, so let's remove the request from it,
                            // even that the notifs thread may have removed it already
                            remove_auth_req_from_list(auth_reqs_handle.clone(), *req_id);
                            let msg = "Failed to get authorisation response";
                            println!("{}", msg);
                            return Ok(Async::Ready(err_response(msg.to_string())));
                        }
                    }
                }
            }
        }
    }
}

enum AuthdResponse {
    NotReady(ProcessRequest),
    Ready(FutureItemType),
}

fn process_authenticator_req(
    req_args: Vec<&str>,
    safe_auth_handle: SharedSafeAuthenticatorHandle,
    auth_reqs_handle: SharedAuthReqsHandle,
    notif_endpoints_handle: SharedNotifEndpointsHandle,
) -> Result<AuthdResponse, FutureErrorType> {
    let action = req_args[1];
    let action_args = &req_args[2..];
    println!("Processing new incoming request: '{}'", action);

    let outcome = match action {
        "status" => status::process_req(
            action_args,
            safe_auth_handle,
            auth_reqs_handle,
            notif_endpoints_handle,
        ),
        "login" => log_in::process_req(action_args, safe_auth_handle),
        "logout" => log_out::process_req(action_args, safe_auth_handle, auth_reqs_handle),
        "create" => create_acc::process_req(action_args, safe_auth_handle),
        "authed-apps" => authed_apps::process_req(action_args, safe_auth_handle),
        "revoke" => revoke::process_req(action_args, safe_auth_handle),
        "auth-reqs" => auth_reqs::process_req(action_args, auth_reqs_handle),
        "allow" => allow::process_req(action_args, auth_reqs_handle),
        "deny" => deny::process_req(action_args, auth_reqs_handle),
        "subscribe" => subscribe::process_req(action_args, notif_endpoints_handle),
        "unsubscribe" => unsubscribe::process_req(action_args, notif_endpoints_handle),
        "authorise" => {
            match authorise::process_req(
                action_args,
                safe_auth_handle.clone(),
                auth_reqs_handle.clone(),
            ) {
                Ok(authorise::AuthorisationResponse::NotReady((rx, req_id, auth_req_str))) => {
                    let processing_resp = ProcessRequest::ProcessingResponse {
                        safe_auth_handle: safe_auth_handle.clone(),
                        auth_reqs_handle: auth_reqs_handle.clone(),
                        rx,
                        req_id,
                        auth_req_str,
                    };
                    return Ok(AuthdResponse::NotReady(processing_resp));
                }
                Ok(authorise::AuthorisationResponse::Ready(resp)) => Ok(resp),
                Err(err) => Err(err),
            }
        }
        other => {
            println!(
                "Action '{}' not supported or unknown by the Authenticator daemon",
                other
            );
            Err("Action not supported or unknown".to_string())
        }
    };

    match outcome {
        Ok(resp_msg) => Ok(AuthdResponse::Ready(successful_response(resp_msg))),
        Err(err_msg) => Ok(AuthdResponse::Ready(err_response(err_msg))),
    }
}
