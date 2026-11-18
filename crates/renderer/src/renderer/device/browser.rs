//! Browser error scopes and device method failures (wasm32 only).

use super::*;

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(in crate::renderer) struct BrowserGpuError {
    pub(in crate::renderer) name: String,
    pub(in crate::renderer) constructor_name: String,
    pub(in crate::renderer) message: String,
}

#[cfg(target_arch = "wasm32")]
impl BrowserGpuError {
    pub(in crate::renderer) fn from_js(error: &wasm_bindgen::JsValue) -> Self {
        use wasm_bindgen::JsValue;

        let string_property = |object: &JsValue, property: &str| {
            js_sys::Reflect::get(object, &JsValue::from_str(property))
                .ok()
                .and_then(|value| value.as_string())
                .filter(|value| !value.is_empty())
        };
        let constructor_name = js_sys::Reflect::get(error, &JsValue::from_str("constructor"))
            .ok()
            .and_then(|constructor| string_property(&constructor, "name"))
            .unwrap_or_else(|| "unknown".into());
        Self {
            name: string_property(error, "name").unwrap_or_else(|| "GPUError".into()),
            constructor_name,
            message: string_property(error, "message").unwrap_or_else(|| format!("{error:?}")),
        }
    }

    pub(in crate::renderer) fn describe(&self) -> String {
        format!(
            "{} (constructor {}): {}",
            self.name, self.constructor_name, self.message
        )
    }
}

#[cfg(target_arch = "wasm32")]
pub(in crate::renderer) fn browser_gpu_device_method(
    device: &wgpu::webgpu::GpuDevice,
    name: &str,
) -> std::result::Result<js_sys::Function, String> {
    use wasm_bindgen::{JsCast, JsValue};

    js_sys::Reflect::get(device.as_ref(), &JsValue::from_str(name))
        .map_err(|error| format!("GPUDevice.{name} lookup failed: {error:?}"))?
        .dyn_into()
        .map_err(|_| format!("GPUDevice.{name} is not a function"))
}

#[cfg(target_arch = "wasm32")]
pub(in crate::renderer) fn browser_scope_failure(
    resource: &'static str,
    reason: String,
) -> FiggyError {
    FiggyError::GpuResourceAllocationFailed { resource, reason }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct BrowserErrorScopes {
    pub(in crate::renderer) device: wgpu::webgpu::GpuDevice,
    pub(in crate::renderer) pop: js_sys::Function,
    pub(in crate::renderer) active: u8,
    pub(in crate::renderer) resource: &'static str,
}

#[cfg(target_arch = "wasm32")]
impl BrowserErrorScopes {
    pub(crate) fn push(device: &wgpu::Device, resource: &'static str) -> Result<Self> {
        use wasm_bindgen::JsValue;

        let device = device.as_webgpu().cloned().ok_or_else(|| {
            browser_scope_failure(
                resource,
                "wgpu device is not backed by a browser GPUDevice".into(),
            )
        })?;
        let push = browser_gpu_device_method(&device, "pushErrorScope")
            .map_err(|reason| browser_scope_failure(resource, reason))?;
        let pop = browser_gpu_device_method(&device, "popErrorScope")
            .map_err(|reason| browser_scope_failure(resource, reason))?;
        let mut scopes = Self {
            device,
            pop,
            active: 0,
            resource,
        };
        for filter in ["out-of-memory", "internal"] {
            push.call1(scopes.device.as_ref(), &JsValue::from_str(filter))
                .map_err(|error| {
                    browser_scope_failure(
                        resource,
                        format!("GPUDevice.pushErrorScope({filter}) failed: {error:?}"),
                    )
                })?;
            scopes.active += 1;
        }
        Ok(scopes)
    }

    pub(in crate::renderer) fn request_pop(
        &mut self,
    ) -> std::result::Result<js_sys::Promise, String> {
        use wasm_bindgen::JsCast;

        if self.active == 0 {
            return Err("no figgy error scope is left to pop".to_owned());
        }
        let value = self
            .pop
            .call0(self.device.as_ref())
            .map_err(|error| format!("GPUDevice.popErrorScope failed: {error:?}"))?;
        // A successful call starts the pop and removes that scope from the
        // device stack even though its Promise is still pending.
        self.active -= 1;
        value
            .dyn_into::<js_sys::Promise>()
            .map_err(|_| "GPUDevice.popErrorScope did not return a Promise".to_owned())
    }

    pub(crate) fn request_all(mut self) -> BrowserPendingErrorScopes {
        // Request both pops before returning to JS. Once popErrorScope has
        // returned its Promise, that scope no longer occupies the device
        // stack while the Promise is awaited.
        let internal = self.request_pop();
        let out_of_memory = self.request_pop();
        BrowserPendingErrorScopes {
            internal,
            out_of_memory,
            resource: self.resource,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserErrorScopes {
    fn drop(&mut self) {
        // Dropping an async export also drops its locals. Start every remaining
        // pop synchronously; JS Promises keep running after their Rust handles
        // are discarded.
        while self.active > 0 {
            self.active -= 1;
            if self.pop.call0(self.device.as_ref()).is_err() {
                // Drop has no caller to report to, and a device that rejects
                // one pop rejects the rest: its error stack is gone (the
                // device was lost), so there is nothing left to pop.
                break;
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct BrowserPendingErrorScopes {
    pub(in crate::renderer) internal: std::result::Result<js_sys::Promise, String>,
    pub(in crate::renderer) out_of_memory: std::result::Result<js_sys::Promise, String>,
    pub(in crate::renderer) resource: &'static str,
}

#[cfg(target_arch = "wasm32")]
impl BrowserPendingErrorScopes {
    pub(crate) async fn finish(self) -> Result<()> {
        let internal = browser_await_error_scope(self.internal).await;
        let out_of_memory = browser_await_error_scope(self.out_of_memory).await;
        finish_browser_error_scopes(self.resource, internal, out_of_memory)
    }
}

#[cfg(target_arch = "wasm32")]
pub(in crate::renderer) async fn browser_await_error_scope(
    request: std::result::Result<js_sys::Promise, String>,
) -> std::result::Result<Option<BrowserGpuError>, String> {
    use wasm_bindgen_futures::JsFuture;

    let error = JsFuture::from(request?)
        .await
        .map_err(|error| format!("GPUDevice.popErrorScope rejected: {error:?}"))?;
    if error.is_null() || error.is_undefined() {
        Ok(None)
    } else {
        Ok(Some(BrowserGpuError::from_js(&error)))
    }
}

#[cfg(target_arch = "wasm32")]
pub(in crate::renderer) fn finish_browser_error_scopes(
    resource: &'static str,
    internal: std::result::Result<Option<BrowserGpuError>, String>,
    out_of_memory: std::result::Result<Option<BrowserGpuError>, String>,
) -> Result<()> {
    let mut failures = Vec::new();
    match internal {
        Ok(Some(error)) => failures.push(format!(
            "browser WebGPU internal error: {}",
            error.describe()
        )),
        Err(error) => failures.push(format!("browser internal scope pop failed: {error}")),
        Ok(None) => {}
    }
    match out_of_memory {
        Ok(Some(error)) => failures.push(format!(
            "browser WebGPU out-of-memory error: {}",
            error.describe()
        )),
        Err(error) => failures.push(format!("browser OOM scope pop failed: {error}")),
        Ok(None) => {}
    }

    if failures.is_empty() {
        return Ok(());
    }
    Err(browser_scope_failure(resource, failures.join("; ")))
}
