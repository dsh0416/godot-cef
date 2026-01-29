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
    remote_debugging_port: u16,
    security_config: SecurityConfig,
    /// GPU device IDs for GPU selection (all platforms)
    gpu_device_ids: Option<GpuDeviceIds>,
    /// Custom user agent string (empty = use CEF default)
    user_agent: String,
    /// Proxy server URL (empty = direct connection)
    proxy_server: String,
    /// Proxy bypass list (empty = no bypass)
    proxy_bypass_list: String,
    /// Cache size limit in MB (0 = use CEF default)
    cache_size_mb: i32,
    /// Custom command-line switches
    custom_switches: Vec<String>,
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
            remote_debugging_port: 9229,
            security_config: SecurityConfig::default(),
            gpu_device_ids: None,
            user_agent: String::new(),
            proxy_server: String::new(),
            proxy_bypass_list: String::new(),
            cache_size_mb: 0,
            custom_switches: Vec::new(),
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

    pub fn remote_debugging_port(&self) -> u16 {
        self.remote_debugging_port
    }

    pub fn security_config(&self) -> &SecurityConfig {
        &self.security_config
    }

    pub fn gpu_device_ids(&self) -> Option<GpuDeviceIds> {
        self.gpu_device_ids
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub fn proxy_server(&self) -> &str {
        &self.proxy_server
    }

    pub fn proxy_bypass_list(&self) -> &str {
        &self.proxy_bypass_list
    }

    pub fn cache_size_mb(&self) -> i32 {
        self.cache_size_mb
    }

    pub fn custom_switches(&self) -> &[String] {
        &self.custom_switches
    }
}

pub struct OsrAppBuilder {
    godot_backend: GodotRenderBackend,
    enable_remote_debugging: bool,
    remote_debugging_port: u16,
    security_config: SecurityConfig,
    gpu_device_ids: Option<GpuDeviceIds>,
    user_agent: String,
    proxy_server: String,
    proxy_bypass_list: String,
    cache_size_mb: i32,
    custom_switches: Vec<String>,
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
            remote_debugging_port: 9229,
            security_config: SecurityConfig::default(),
            gpu_device_ids: None,
            user_agent: String::new(),
            proxy_server: String::new(),
            proxy_bypass_list: String::new(),
            cache_size_mb: 0,
            custom_switches: Vec::new(),
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

    pub fn remote_debugging_port(mut self, port: u16) -> Self {
        self.remote_debugging_port = port;
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

    pub fn user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    pub fn proxy_server(mut self, proxy_server: String) -> Self {
        self.proxy_server = proxy_server;
        self
    }

    pub fn proxy_bypass_list(mut self, proxy_bypass_list: String) -> Self {
        self.proxy_bypass_list = proxy_bypass_list;
        self
    }

    pub fn cache_size_mb(mut self, cache_size_mb: i32) -> Self {
        self.cache_size_mb = cache_size_mb;
        self
    }

    pub fn custom_switches(mut self, custom_switches: Vec<String>) -> Self {
        self.custom_switches = custom_switches;
        self
    }

    pub fn build(self) -> OsrApp {
        OsrApp {
            godot_backend: self.godot_backend,
            enable_remote_debugging: self.enable_remote_debugging,
            remote_debugging_port: self.remote_debugging_port,
            security_config: self.security_config,
            gpu_device_ids: self.gpu_device_ids,
            user_agent: self.user_agent,
            proxy_server: self.proxy_server,
            proxy_bypass_list: self.proxy_bypass_list,
            cache_size_mb: self.cache_size_mb,
            custom_switches: self.custom_switches,
        }
    }
}
