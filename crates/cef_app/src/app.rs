#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GodotRenderBackend {
    #[default]
    Unknown,
    Direct3D12,
    Metal,
    Vulkan,
}

#[derive(Clone, Default)]
pub struct SecurityConfig {
    /// Allow loading insecure (HTTP) content in HTTPS pages.
    pub allow_insecure_content: bool,
    /// Ignore SSL/TLS certificate errors.
    pub ignore_certificate_errors: bool,
    /// Disable web security (CORS, same-origin policy).
    pub disable_web_security: bool,
}

/// GPU device identifiers for GPU selection across all platforms.
///
/// These vendor and device IDs are passed to CEF via `--gpu-vendor-id` and
/// `--gpu-device-id` command-line switches to ensure CEF uses the same GPU as Godot.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuDeviceIds {
    pub vendor_id: u32,
    pub device_id: u32,
}

impl GpuDeviceIds {
    pub fn new(vendor_id: u32, device_id: u32) -> Self {
        Self {
            vendor_id,
            device_id,
        }
    }

    /// Format vendor ID as decimal string for command line argument
    pub fn to_vendor_arg(&self) -> String {
        format!("{}", self.vendor_id)
    }

    /// Format device ID as decimal string for command line argument
    pub fn to_device_arg(&self) -> String {
        format!("{}", self.device_id)
    }
}

#[derive(Clone)]
pub struct OsrApp {
    godot_backend: GodotRenderBackend,
    enable_remote_debugging: bool,
    security_config: SecurityConfig,
    /// GPU device IDs for GPU selection (all platforms)
    gpu_device_ids: Option<GpuDeviceIds>,
}

impl Default for OsrApp {
    fn default() -> Self {
        Self::new()
    }
}

impl OsrApp {
    pub fn new() -> Self {
        Self {
            godot_backend: GodotRenderBackend::Unknown,
            enable_remote_debugging: false,
            security_config: SecurityConfig::default(),
            gpu_device_ids: None,
        }
    }

    pub fn builder() -> OsrAppBuilder {
        OsrAppBuilder::new()
    }

    pub fn godot_backend(&self) -> GodotRenderBackend {
        self.godot_backend
    }

    pub fn enable_remote_debugging(&self) -> bool {
        self.enable_remote_debugging
    }

    pub fn security_config(&self) -> &SecurityConfig {
        &self.security_config
    }

    pub fn gpu_device_ids(&self) -> Option<GpuDeviceIds> {
        self.gpu_device_ids
    }
}

pub struct OsrAppBuilder {
    godot_backend: GodotRenderBackend,
    enable_remote_debugging: bool,
    security_config: SecurityConfig,
    gpu_device_ids: Option<GpuDeviceIds>,
}

impl Default for OsrAppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl OsrAppBuilder {
    pub fn new() -> Self {
        Self {
            godot_backend: GodotRenderBackend::Unknown,
            enable_remote_debugging: false,
            security_config: SecurityConfig::default(),
            gpu_device_ids: None,
        }
    }

    pub fn godot_backend(mut self, godot_backend: GodotRenderBackend) -> Self {
        self.godot_backend = godot_backend;
        self
    }

    pub fn remote_debugging(mut self, enable_remote_debugging: bool) -> Self {
        self.enable_remote_debugging = enable_remote_debugging;
        self
    }

    pub fn security_config(mut self, security_config: SecurityConfig) -> Self {
        self.security_config = security_config;
        self
    }

    pub fn gpu_device_ids(mut self, vendor_id: u32, device_id: u32) -> Self {
        self.gpu_device_ids = Some(GpuDeviceIds::new(vendor_id, device_id));
        self
    }

    pub fn build(self) -> OsrApp {
        OsrApp {
            godot_backend: self.godot_backend,
            enable_remote_debugging: self.enable_remote_debugging,
            security_config: self.security_config,
            gpu_device_ids: self.gpu_device_ids,
        }
    }
}
