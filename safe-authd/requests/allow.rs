// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::shared::{lock_auth_reqs_list, SharedAuthReqsHandle};

pub fn process_req(
    args: &[&str],
    auth_reqs_handle: SharedAuthReqsHandle,
) -> Result<String, String> {
    if args.len() != 1 {
        Err("Incorrect number of arguments for 'allow' action".to_string())
    } else {
        println!("Allowing authorisation request...");
        let auth_req_id = args[0];
        let req_id = match auth_req_id.parse::<u32>() {
            Ok(id) => id,
            Err(err) => return Err(err.to_string()),
        };

        lock_auth_reqs_list(auth_reqs_handle, |auth_reqs_list| {
            match auth_reqs_list.remove(&req_id) {
                Some(mut auth_req) => match auth_req.tx.try_send(true) {
                    Ok(_) => {
                        let msg = format!(
                            "Authorisation request ({}) allowed successfully",
                            auth_req_id
                        );
                        println!("{}", msg);
                        Ok(msg)
                    }
                    Err(_) => {
                        let msg = format!("Failed to allow authorisation request '{}' since the response couldn't be sent to the requesting application", auth_req_id);
                        println!("{}", msg);
                        Err(msg)
                    }
                },
                None => {
                    let msg = format!(
                        "No pending authorisation request found with id '{}'",
                        auth_req_id
                    );
                    println!("{}", msg);
                    Err(msg)
                }
            }
        })
    }
}
