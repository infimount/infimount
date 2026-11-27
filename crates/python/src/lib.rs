//! Thin, explicitly configured Python access to governed storage operations.
mod client;
mod config;
mod error;
mod local;

use std::sync::Arc;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyBytesMethods};

use crate::error::Error;

create_exception!(_native, InfimountError, PyException);
create_exception!(_native, ConfigurationError, InfimountError);
create_exception!(_native, InvalidRequest, InfimountError);
create_exception!(_native, PermissionDenied, InfimountError);
create_exception!(_native, ConfirmationRequired, InfimountError);
create_exception!(_native, NotFound, InfimountError);
create_exception!(_native, AlreadyExists, InfimountError);
create_exception!(_native, LimitExceeded, InfimountError);
create_exception!(_native, UnsupportedOperation, InfimountError);
create_exception!(_native, BackendUnavailable, InfimountError);

fn python_error(error: Error) -> PyErr {
    let message = error.code();
    match error {
        Error::Configuration => ConfigurationError::new_err(message),
        Error::InvalidRequest => InvalidRequest::new_err(message),
        Error::PermissionDenied => PermissionDenied::new_err(message),
        Error::ConfirmationRequired => ConfirmationRequired::new_err(message),
        Error::NotFound => NotFound::new_err(message),
        Error::AlreadyExists => AlreadyExists::new_err(message),
        Error::LimitExceeded => LimitExceeded::new_err(message),
        Error::Unsupported => UnsupportedOperation::new_err(message),
        Error::Unavailable => BackendUnavailable::new_err(message),
        Error::Closed | Error::Forked => InfimountError::new_err(message),
    }
}

#[pyclass(frozen, name = "NativeClient", module = "infimount._native")]
struct NativeClient {
    inner: Arc<client::Client>,
}

#[pymethods]
impl NativeClient {
    #[new]
    fn new(py: Python<'_>, config_json: &str) -> PyResult<Self> {
        let inner = py
            .detach(|| client::Client::new(config_json))
            .map_err(python_error)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    #[pyo3(signature = (path, *, limit=200, cursor=None))]
    fn list_json(
        &self,
        py: Python<'_>,
        path: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> PyResult<String> {
        py.detach(|| self.inner.list(path, limit, cursor))
            .map_err(python_error)
    }

    fn stat_json(&self, py: Python<'_>, path: &str) -> PyResult<String> {
        py.detach(|| self.inner.stat(path)).map_err(python_error)
    }

    #[pyo3(signature = (path, *, offset=0, max_bytes=None))]
    fn read<'py>(
        &self,
        py: Python<'py>,
        path: &str,
        offset: u64,
        max_bytes: Option<u64>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = py
            .detach(|| self.inner.read(path, offset, max_bytes))
            .map_err(python_error)?;
        PyBytes::new_with(py, bytes.len(), |destination| {
            destination.copy_from_slice(&bytes);
            Ok(())
        })
    }

    fn write(&self, py: Python<'_>, path: &str, data: &Bound<'_, PyBytes>) -> PyResult<u64> {
        // Check the immutable Python buffer before making a Rust copy.
        if data.as_bytes().len() > self.inner.limits.write_bytes {
            return Err(python_error(self.inner.reject_oversized_write(path)));
        }
        let bytes = data.as_bytes().to_vec();
        py.detach(|| self.inner.write(path, bytes))
            .map_err(python_error)
    }

    fn delete(&self, py: Python<'_>, path: &str) -> PyResult<()> {
        py.detach(|| self.inner.delete(path)).map_err(python_error)
    }

    #[pyo3(signature = (path, *, expires_seconds=300))]
    fn download_link_json(
        &self,
        py: Python<'_>,
        path: &str,
        expires_seconds: u64,
    ) -> PyResult<String> {
        py.detach(|| self.inner.download_link(path, expires_seconds))
            .map_err(python_error)
    }

    fn drain_audit_json(&self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| self.inner.audit_json()).map_err(python_error)
    }

    fn close(&self) {
        self.inner.close();
    }

    fn __repr__(&self) -> String {
        self.inner.description()
    }

    fn __getstate__(&self) -> PyResult<()> {
        Err(PyTypeError::new_err("Infimount clients cannot be pickled"))
    }
}

#[pymodule(gil_used = true)]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeClient>()?;
    let py = module.py();
    module.add("InfimountError", py.get_type::<InfimountError>())?;
    module.add("ConfigurationError", py.get_type::<ConfigurationError>())?;
    module.add("InvalidRequest", py.get_type::<InvalidRequest>())?;
    module.add("PermissionDenied", py.get_type::<PermissionDenied>())?;
    module.add(
        "ConfirmationRequired",
        py.get_type::<ConfirmationRequired>(),
    )?;
    module.add("NotFound", py.get_type::<NotFound>())?;
    module.add("AlreadyExists", py.get_type::<AlreadyExists>())?;
    module.add("LimitExceeded", py.get_type::<LimitExceeded>())?;
    module.add(
        "UnsupportedOperation",
        py.get_type::<UnsupportedOperation>(),
    )?;
    module.add("BackendUnavailable", py.get_type::<BackendUnavailable>())?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
