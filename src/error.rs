use std::{borrow::Cow, error::Error};

use serde_json::Value;

pub(crate) fn map_err_to_internal_error(e: impl Error, msg: String) -> tower_lsp::jsonrpc::Error {
    let mut err = tower_lsp::jsonrpc::Error::internal_error();
    err.data = Some(Value::String(format!("{e:?}")));
    err.message = Cow::from(msg);
    err
}

pub(crate) fn map_err_to_parse_error(e: impl Error, msg: String) -> tower_lsp::jsonrpc::Error {
    let mut err = tower_lsp::jsonrpc::Error::parse_error();
    err.data = Some(Value::String(format!("{e:?}")));
    err.message = Cow::from(msg);
    err
}
