// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::errors::Result;
use super::ffi_structs::{processed_files_into_repr_c, ProcessedFiles};
use super::helpers::from_c_str_to_str_option;
use ffi_utils::{
    catch_unwind_cb, vec_clone_from_raw_parts, FfiResult, OpaqueCtx, ReprC, FFI_RESULT_OK,
};
use safe_api::Safe;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

#[no_mangle]
pub unsafe extern "C" fn files_container_create(
    app: *mut Safe,
    location: *const c_char,
    dest: *const c_char,
    recursive: bool,
    dry_run: bool,
    user_data: *mut c_void,
    o_cb: extern "C" fn(
        user_data: *mut c_void,
        result: *const FfiResult,
        xorurl: *const c_char,
        process_files: *const ProcessedFiles,
        files_map: *const c_char,
    ),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let location_str = String::clone_from_repr_c(location)?;
        let destination = from_c_str_to_str_option(dest);
        let (xorurl, processed_files, files_map) =
            (*app).files_container_create(&location_str, destination, recursive, dry_run)?;
        let xorurl_string = CString::new(xorurl)?;
        let files_map_json = CString::new(serde_json::to_string(&files_map)?)?;
        let ffi_processed_files = processed_files_into_repr_c(&processed_files)?;
        o_cb(
            user_data.0,
            FFI_RESULT_OK,
            xorurl_string.as_ptr(),
            &ffi_processed_files,
            files_map_json.as_ptr(),
        );
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn files_container_get(
    app: *mut Safe,
    url: *const c_char,
    user_data: *mut c_void,
    o_cb: extern "C" fn(
        user_data: *mut c_void,
        result: *const FfiResult,
        version: u64,
        files_map: *const c_char,
    ),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let url_str = String::clone_from_repr_c(url)?;
        let (version, files_map) = (*app).files_container_get(&url_str)?;
        let files_map_json = CString::new(serde_json::to_string(&files_map)?)?;
        o_cb(user_data.0, FFI_RESULT_OK, version, files_map_json.as_ptr());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn files_container_sync(
    app: *mut Safe,
    location: *const c_char,
    url: *const c_char,
    recursive: bool,
    delete: bool,
    update_nrs: bool,
    dry_run: bool,
    user_data: *mut c_void,
    o_cb: extern "C" fn(
        user_data: *mut c_void,
        result: *const FfiResult,
        version: u64,
        process_files: *const ProcessedFiles,
        files_map: *const c_char,
    ),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let location_str = String::clone_from_repr_c(location)?;
        let url_str = String::clone_from_repr_c(url)?;
        let (version, processed_files, files_map) = (*app).files_container_sync(
            &location_str,
            &url_str,
            recursive,
            delete,
            update_nrs,
            dry_run,
        )?;
        let files_map_json = CString::new(serde_json::to_string(&files_map)?)?;
        let ffi_processed_files = processed_files_into_repr_c(&processed_files)?;
        o_cb(
            user_data.0,
            FFI_RESULT_OK,
            version,
            &ffi_processed_files,
            files_map_json.as_ptr(),
        );
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn files_container_add(
    app: *mut Safe,
    source_file: *const c_char,
    url: *const c_char,
    force: bool,
    update_nrs: bool,
    dry_run: bool,
    user_data: *mut c_void,
    o_cb: extern "C" fn(
        user_data: *mut c_void,
        result: *const FfiResult,
        version: u64,
        process_files: *const ProcessedFiles,
        files_map: *const c_char,
    ),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let url_str = String::clone_from_repr_c(url)?;
        let source_str = String::clone_from_repr_c(source_file)?;
        let (version, processed_files, files_map) =
            (*app).files_container_add(&source_str, &url_str, force, update_nrs, dry_run)?;
        let files_map_json = CString::new(serde_json::to_string(&files_map)?)?;
        let ffi_processed_files = processed_files_into_repr_c(&processed_files)?;
        o_cb(
            user_data.0,
            FFI_RESULT_OK,
            version,
            &ffi_processed_files,
            files_map_json.as_ptr(),
        );
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn files_container_add_from_raw(
    app: *mut Safe,
    data: *const u8,
    data_len: usize,
    url: *const c_char,
    force: bool,
    update_nrs: bool,
    dry_run: bool,
    user_data: *mut c_void,
    o_cb: extern "C" fn(
        user_data: *mut c_void,
        result: *const FfiResult,
        version: u64,
        process_files: *const ProcessedFiles,
        files_map: *const c_char,
    ),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let data_vec = vec_clone_from_raw_parts(data, data_len);
        let url_str = String::clone_from_repr_c(url)?;
        let (version, processed_files, files_map) =
            (*app).files_container_add_from_raw(&data_vec, &url_str, force, update_nrs, dry_run)?;
        let files_map_json = CString::new(serde_json::to_string(&files_map)?)?;
        let ffi_processed_files = processed_files_into_repr_c(&processed_files)?;
        o_cb(
            user_data.0,
            FFI_RESULT_OK,
            version,
            &ffi_processed_files,
            files_map_json.as_ptr(),
        );
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn files_put_published_immutable(
    app: *mut Safe,
    data: *const u8,
    data_len: usize,
    media_type: *const c_char,
    user_data: *mut c_void,
    o_cb: extern "C" fn(user_data: *mut c_void, result: *const FfiResult, xorurl: *const c_char),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let media_type_str = from_c_str_to_str_option(media_type);
        let data_vec = vec_clone_from_raw_parts(data, data_len);
        let xorurl = (*app).files_put_published_immutable(&data_vec, media_type_str)?;
        let xorurl_string = CString::new(xorurl)?;
        o_cb(user_data.0, FFI_RESULT_OK, xorurl_string.as_ptr());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn files_get_published_immutable(
    app: *mut Safe,
    url: *const c_char,
    user_data: *mut c_void,
    o_cb: extern "C" fn(
        user_data: *mut c_void,
        result: *const FfiResult,
        im_data: *const u8,
        im_data_len: usize,
    ),
) {
    catch_unwind_cb(user_data, o_cb, || -> Result<()> {
        let user_data = OpaqueCtx(user_data);
        let url_str = String::clone_from_repr_c(url)?;
        let data = (*app).files_get_published_immutable(&url_str)?;
        o_cb(user_data.0, FFI_RESULT_OK, data.as_ptr(), data.len());
        Ok(())
    })
}
