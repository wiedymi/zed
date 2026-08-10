use std::{ptr::NonNull, slice};

use anyhow::{Context as _, Result, bail, ensure};
use ohos_sys::asset_store::{asset_api as api, asset_type as asset};
use sha2::{Digest as _, Sha256};

const CREDENTIAL_FORMAT_VERSION: u8 = 1;
const MAX_SECRET_LENGTH: usize = 1024;

pub(crate) fn write(url: &str, username: &str, password: &[u8]) -> Result<()> {
    let alias = credential_alias(url);
    let secret = encode_secret(username, password)?;
    let attributes = [
        bytes_attribute(asset::Asset_Tag::ASSET_TAG_ALIAS, &alias)?,
        bytes_attribute(asset::Asset_Tag::ASSET_TAG_SECRET, &secret)?,
        number_attribute(
            asset::Asset_Tag::ASSET_TAG_ACCESSIBILITY,
            asset::Asset_Accessibility::ASSET_ACCESSIBILITY_DEVICE_FIRST_UNLOCKED.0,
        ),
        number_attribute(
            asset::Asset_Tag::ASSET_TAG_SYNC_TYPE,
            asset::Asset_SyncType::ASSET_SYNC_TYPE_NEVER.0,
        ),
        number_attribute(
            asset::Asset_Tag::ASSET_TAG_CONFLICT_RESOLUTION,
            asset::Asset_ConflictResolution::ASSET_CONFLICT_OVERWRITE.0,
        ),
        boolean_attribute(asset::Asset_Tag::ASSET_TAG_IS_PERSISTENT, true),
        boolean_attribute(asset::Asset_Tag::ASSET_TAG_REQUIRE_ATTR_ENCRYPTED, true),
    ];

    // SAFETY: Every attribute points into `alias` or `secret`, both of which
    // remain alive until the synchronous Asset Store call returns.
    let status = unsafe { api::OH_Asset_Add(attributes.as_ptr(), attributes.len() as u32) };
    ensure_success("writing credentials", status)
}

pub(crate) fn read(url: &str) -> Result<Option<(String, Vec<u8>)>> {
    let alias = credential_alias(url);
    let query = [
        bytes_attribute(asset::Asset_Tag::ASSET_TAG_ALIAS, &alias)?,
        boolean_attribute(asset::Asset_Tag::ASSET_TAG_IS_PERSISTENT, true),
        boolean_attribute(asset::Asset_Tag::ASSET_TAG_REQUIRE_ATTR_ENCRYPTED, true),
        number_attribute(
            asset::Asset_Tag::ASSET_TAG_RETURN_TYPE,
            asset::Asset_ReturnType::ASSET_RETURN_ALL.0,
        ),
    ];
    let mut result_set = asset::Asset_ResultSet {
        count: 0,
        results: std::ptr::null_mut(),
    };
    // SAFETY: The query points into the live `alias` buffer, and `result_set`
    // is a valid out parameter for the synchronous call.
    let status =
        unsafe { api::OH_Asset_Query(query.as_ptr(), query.len() as u32, &mut result_set) };
    if status == asset::Asset_ResultCode::ASSET_NOT_FOUND.0 as i32 {
        return Ok(None);
    }
    ensure_success("reading credentials", status)?;
    let result_set = ResultSet(result_set);
    if result_set.0.count == 0 {
        return Ok(None);
    }
    ensure!(
        result_set.0.count == 1,
        "HarmonyOS Asset Store returned {} credentials for one alias",
        result_set.0.count
    );
    let result = NonNull::new(result_set.0.results)
        .context("HarmonyOS Asset Store returned a null credential result")?;
    // SAFETY: A successful query reported exactly one result, so the first
    // element exists until `result_set` is dropped.
    let secret_attribute =
        unsafe { api::OH_Asset_ParseAttr(result.as_ptr(), asset::Asset_Tag::ASSET_TAG_SECRET) };
    let secret_attribute =
        NonNull::new(secret_attribute).context("HarmonyOS Asset Store credential has no secret")?;
    // SAFETY: The requested tag is a byte-valued secret attribute.
    let secret = unsafe { secret_attribute.as_ref().value.blob };
    let secret_length = secret.size as usize;
    ensure!(
        secret_length <= MAX_SECRET_LENGTH,
        "HarmonyOS Asset Store returned an oversized credential secret"
    );
    let secret_data = NonNull::new(secret.data)
        .filter(|_| secret_length > 0)
        .context("HarmonyOS Asset Store returned an empty credential secret")?;
    // SAFETY: The Asset Store owns `secret_length` initialized bytes at this
    // pointer until `result_set` is freed below.
    let secret = unsafe { slice::from_raw_parts(secret_data.as_ptr(), secret_length) };
    let credentials = decode_secret(secret)?;
    drop(result_set);
    Ok(Some(credentials))
}

pub(crate) fn delete(url: &str) -> Result<()> {
    let alias = credential_alias(url);
    let query = [
        bytes_attribute(asset::Asset_Tag::ASSET_TAG_ALIAS, &alias)?,
        boolean_attribute(asset::Asset_Tag::ASSET_TAG_IS_PERSISTENT, true),
        boolean_attribute(asset::Asset_Tag::ASSET_TAG_REQUIRE_ATTR_ENCRYPTED, true),
    ];
    // SAFETY: The query points into `alias`, which remains live until this
    // synchronous call returns.
    let status = unsafe { api::OH_Asset_Remove(query.as_ptr(), query.len() as u32) };
    if status == asset::Asset_ResultCode::ASSET_NOT_FOUND.0 as i32 {
        return Ok(());
    }
    ensure_success("deleting credentials", status)
}

fn credential_alias(url: &str) -> Vec<u8> {
    format!(
        "zed-credentials-v1:{}",
        hex::encode(Sha256::digest(url.as_bytes()))
    )
    .into_bytes()
}

fn encode_secret(username: &str, password: &[u8]) -> Result<Vec<u8>> {
    let username_length =
        u32::try_from(username.len()).context("credential username is too long")?;
    let total_length = 1usize
        .checked_add(size_of::<u32>())
        .and_then(|length| length.checked_add(username.len()))
        .and_then(|length| length.checked_add(password.len()))
        .context("credential size overflow")?;
    ensure!(
        total_length <= MAX_SECRET_LENGTH,
        "credential is {total_length} bytes, which exceeds the HarmonyOS Asset Store limit of {MAX_SECRET_LENGTH} bytes"
    );

    let mut secret = Vec::with_capacity(total_length);
    secret.push(CREDENTIAL_FORMAT_VERSION);
    secret.extend_from_slice(&username_length.to_le_bytes());
    secret.extend_from_slice(username.as_bytes());
    secret.extend_from_slice(password);
    Ok(secret)
}

fn decode_secret(secret: &[u8]) -> Result<(String, Vec<u8>)> {
    let (&version, body) = secret
        .split_first()
        .context("credential secret has no format version")?;
    ensure!(
        version == CREDENTIAL_FORMAT_VERSION,
        "unsupported credential format version {version}"
    );
    let username_length_bytes: [u8; size_of::<u32>()] = body
        .get(..size_of::<u32>())
        .context("credential secret has no username length")?
        .try_into()
        .context("credential username length is malformed")?;
    let username_length = u32::from_le_bytes(username_length_bytes) as usize;
    let username_start = size_of::<u32>();
    let username_end = username_start
        .checked_add(username_length)
        .context("credential username length overflow")?;
    let username = body
        .get(username_start..username_end)
        .context("credential username exceeds the stored secret")?;
    let username = std::str::from_utf8(username)
        .context("credential username is not valid UTF-8")?
        .to_owned();
    let password = body
        .get(username_end..)
        .context("credential password offset exceeds the stored secret")?
        .to_vec();
    Ok((username, password))
}

fn bytes_attribute(tag: asset::Asset_Tag, value: &[u8]) -> Result<asset::Asset_Attr> {
    let size = u32::try_from(value.len()).context("Asset Store byte attribute is too long")?;
    Ok(asset::Asset_Attr {
        tag: tag.0,
        value: asset::Asset_Value {
            blob: asset::Asset_Blob {
                size,
                data: value.as_ptr().cast_mut(),
            },
        },
    })
}

fn number_attribute(tag: asset::Asset_Tag, value: u32) -> asset::Asset_Attr {
    asset::Asset_Attr {
        tag: tag.0,
        value: asset::Asset_Value { u32_: value },
    }
}

fn boolean_attribute(tag: asset::Asset_Tag, value: bool) -> asset::Asset_Attr {
    asset::Asset_Attr {
        tag: tag.0,
        value: asset::Asset_Value { boolean: value },
    }
}

fn ensure_success(operation: &str, status: i32) -> Result<()> {
    if status != asset::Asset_ResultCode::ASSET_SUCCESS.0 as i32 {
        bail!("{operation} failed with HarmonyOS Asset Store status {status}");
    }
    Ok(())
}

struct ResultSet(asset::Asset_ResultSet);

impl Drop for ResultSet {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns the successful query's result set and frees
        // it exactly once.
        unsafe { api::OH_Asset_FreeResultSet(&mut self.0) };
    }
}
