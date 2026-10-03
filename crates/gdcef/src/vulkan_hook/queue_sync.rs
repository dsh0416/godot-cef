//! Host synchronization shared by Godot's loader dispatch and native OSR copies.
//!
//! Godot can expose several virtual queues backed by the same VkQueue. Locking
//! only our importer or moving it to the render thread does not synchronize its
//! background transfer submissions. Intercept both Vulkan proc resolvers and
//! exported entry points, then serialize operations on each real VkQueue and
//! exclude every queue during device idle. This is CPU synchronization only.
//!
//! Godot 4.5 initializes extensions at Core before creating DisplayServer
//! (main/main.cpp). Its volkLoadInstance loads device commands through GIPA
//! (thirdparty/volk/volk.c); swapchain presentation is also loaded through GDPA
//! (drivers/vulkan/rendering_device_driver_vulkan.cpp). Both routes matter.

#[cfg(target_arch = "x86_64")]
pub(crate) use hooks::{ensure_queue_synchronization, install, register_created_device};

#[cfg(not(target_arch = "x86_64"))]
pub(crate) fn ensure_queue_synchronization(
    _device: ash::vk::Device,
    _queue: ash::vk::Queue,
) -> Result<(), String> {
    Err("Vulkan queue interception requires an x86_64 host".to_string())
}

#[cfg(target_arch = "x86_64")]
mod hooks {
    use ash::vk::{self, Handle};
    use retour::{Function, GenericDetour};
    use std::collections::HashMap;
    use std::ffi::{CStr, c_char};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
    use std::thread::ThreadId;

    // retour's Function trait cannot represent ash's higher-ranked lifetime
    // aliases. Vulkan passes raw ABI pointers; these monomorphic aliases do not
    // dereference, retain, or extend the lifetime of the borrowed structures.
    type SubmitFn = unsafe extern "system" fn(
        vk::Queue,
        u32,
        *const vk::SubmitInfo<'static>,
        vk::Fence,
    ) -> vk::Result;
    type Submit2Fn = unsafe extern "system" fn(
        vk::Queue,
        u32,
        *const vk::SubmitInfo2<'static>,
        vk::Fence,
    ) -> vk::Result;
    type BindSparseFn = unsafe extern "system" fn(
        vk::Queue,
        u32,
        *const vk::BindSparseInfo<'static>,
        vk::Fence,
    ) -> vk::Result;
    type PresentFn =
        unsafe extern "system" fn(vk::Queue, *const vk::PresentInfoKHR<'static>) -> vk::Result;
    type GetQueue2Fn =
        unsafe extern "system" fn(vk::Device, *const vk::DeviceQueueInfo2<'static>, *mut vk::Queue);
    type DestroyDeviceFn =
        unsafe extern "system" fn(vk::Device, *const vk::AllocationCallbacks<'static>);

    static INSTALL_LOCK: Mutex<()> = Mutex::new(());
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static LIBRARY: OnceLock<libloading::Library> = OnceLock::new();
    static CALLBACK_LIBRARY: OnceLock<libloading::Library> = OnceLock::new();
    static CREATE_DEVICE_WRAPPER: OnceLock<vk::PFN_vkCreateDevice> = OnceLock::new();
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

    #[derive(Default)]
    struct HostState {
        device_owner: Option<(ThreadId, usize)>,
        queues: HashMap<u64, (ThreadId, usize)>,
    }

    #[derive(Default)]
    struct DeviceHostSync {
        state: Mutex<HostState>,
        changed: Condvar,
    }

    struct HostGuard {
        sync: Arc<DeviceHostSync>,
        queue: Option<u64>,
    }

    impl HostGuard {
        fn acquire(sync: Arc<DeviceHostSync>, queue: Option<u64>) -> Self {
            let owner = std::thread::current().id();
            let mut state = sync
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                let device_available = state.device_owner.is_none_or(|(thread, _)| thread == owner);
                let queues_available = match queue {
                    Some(queue) => state
                        .queues
                        .get(&queue)
                        .is_none_or(|(thread, _)| *thread == owner),
                    None => state.queues.values().all(|(thread, _)| *thread == owner),
                };
                if device_available && queues_available {
                    break;
                }
                state = sync
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            match queue {
                Some(queue) => state.queues.entry(queue).or_insert((owner, 0)).1 += 1,
                None => state.device_owner.get_or_insert((owner, 0)).1 += 1,
            }
            drop(state);
            Self { sync, queue }
        }

        fn queue(queue: vk::Queue) -> Self {
            let sync = {
                let mut registry = registry();
                let device = registry.queues.get(&queue.as_raw()).copied().unwrap_or(0);
                Arc::clone(registry.host_devices.entry(device).or_default())
            };
            Self::acquire(sync, Some(queue.as_raw()))
        }

        fn device(device: vk::Device) -> Self {
            let sync = Arc::clone(registry().host_devices.entry(device.as_raw()).or_default());
            Self::acquire(sync, None)
        }
    }

    impl Drop for HostGuard {
        fn drop(&mut self) {
            let mut state = self
                .sync
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match self.queue {
                Some(queue) => {
                    if let Some((_, depth)) = state.queues.get_mut(&queue) {
                        *depth -= 1;
                        if *depth == 0 {
                            state.queues.remove(&queue);
                        }
                    }
                }
                None => {
                    if let Some((_, depth)) = state.device_owner.as_mut() {
                        *depth -= 1;
                        if *depth == 0 {
                            state.device_owner = None;
                        }
                    }
                }
            }
            self.sync.changed.notify_all();
        }
    }

    #[derive(Clone, Copy)]
    struct Dispatch {
        submit: SubmitFn,
        submit2: Option<Submit2Fn>,
        submit2_khr: Option<Submit2Fn>,
        bind_sparse: BindSparseFn,
        present: Option<PresentFn>,
        queue_idle: vk::PFN_vkQueueWaitIdle,
        device_idle: vk::PFN_vkDeviceWaitIdle,
        get_queue: vk::PFN_vkGetDeviceQueue,
        get_queue2: Option<GetQueue2Fn>,
        destroy: DestroyDeviceFn,
    }

    struct DeviceRecord {
        dispatch: Dispatch,
        created_through_hook: bool,
    }

    #[derive(Default)]
    struct Registry {
        devices: HashMap<u64, DeviceRecord>,
        queues: HashMap<u64, u64>,
        host_devices: HashMap<u64, Arc<DeviceHostSync>>,
    }

    fn registry() -> MutexGuard<'static, Registry> {
        REGISTRY
            .get_or_init(|| Mutex::new(Registry::default()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn queue_dispatch(queue: vk::Queue) -> Option<Dispatch> {
        let registry = registry();
        let device = registry.queues.get(&queue.as_raw())?;
        Some(registry.devices.get(device)?.dispatch)
    }

    fn device_dispatch(device: vk::Device) -> Option<Dispatch> {
        registry()
            .devices
            .get(&device.as_raw())
            .map(|record| record.dispatch)
    }

    struct ExportHook<T: Function> {
        address: usize,
        detour: GenericDetour<T>,
    }

    // Returning a trampoline when GDPA aliases the exported function prevents
    // resolver wrappers from recursively calling their own exported detour.
    macro_rules! original {
        ($hook:ident, $candidate:expr, $ty:ty) => {{
            let candidate: Option<$ty> = $candidate;
            match ($hook.get(), candidate) {
                (Some(hook), Some(candidate)) if candidate as usize != hook.address => {
                    Some(candidate)
                }
                (Some(hook), _) => Some(unsafe {
                    std::mem::transmute::<*const (), $ty>(hook.detour.trampoline() as *const ())
                }),
                (None, candidate) => candidate,
            }
        }};
    }

    macro_rules! queue_operation {
        ($static:ident, $function:ident, $ty:ty, $field:ident, required,
         ($queue:ident: vk::Queue $(, $arg:ident: $arg_ty:ty)*)) => {
            static $static: OnceLock<ExportHook<$ty>> = OnceLock::new();
            unsafe extern "system" fn $function($queue: vk::Queue, $($arg: $arg_ty),*) -> vk::Result {
                let _guard = HostGuard::queue($queue);
                let candidate = queue_dispatch($queue).map(|dispatch| dispatch.$field);
                let Some(function) = original!($static, candidate, $ty) else {
                    return vk::Result::ERROR_INITIALIZATION_FAILED;
                };
                unsafe { function($queue, $($arg),*) }
            }
        };
        ($static:ident, $function:ident, $ty:ty, $field:ident, optional,
         ($queue:ident: vk::Queue $(, $arg:ident: $arg_ty:ty)*)) => {
            static $static: OnceLock<ExportHook<$ty>> = OnceLock::new();
            unsafe extern "system" fn $function($queue: vk::Queue, $($arg: $arg_ty),*) -> vk::Result {
                let _guard = HostGuard::queue($queue);
                let candidate = queue_dispatch($queue).and_then(|dispatch| dispatch.$field);
                let Some(function) = original!($static, candidate, $ty) else {
                    return vk::Result::ERROR_INITIALIZATION_FAILED;
                };
                unsafe { function($queue, $($arg),*) }
            }
        };
    }

    queue_operation!(SUBMIT, queue_submit, SubmitFn, submit, required,
        (queue: vk::Queue, count: u32, submits: *const vk::SubmitInfo<'static>, fence: vk::Fence));
    queue_operation!(SUBMIT2, queue_submit2, Submit2Fn, submit2, optional,
        (queue: vk::Queue, count: u32, submits: *const vk::SubmitInfo2<'static>, fence: vk::Fence));
    queue_operation!(SUBMIT2_KHR, queue_submit2_khr, Submit2Fn, submit2_khr, optional,
        (queue: vk::Queue, count: u32, submits: *const vk::SubmitInfo2<'static>, fence: vk::Fence));
    queue_operation!(BIND_SPARSE, queue_bind_sparse, BindSparseFn, bind_sparse, required,
        (queue: vk::Queue, count: u32, binds: *const vk::BindSparseInfo<'static>, fence: vk::Fence));
    queue_operation!(PRESENT, queue_present, PresentFn, present, optional,
        (queue: vk::Queue, info: *const vk::PresentInfoKHR<'static>));
    queue_operation!(QUEUE_IDLE, queue_wait_idle, vk::PFN_vkQueueWaitIdle, queue_idle, required,
        (queue: vk::Queue));

    static DEVICE_IDLE: OnceLock<ExportHook<vk::PFN_vkDeviceWaitIdle>> = OnceLock::new();
    unsafe extern "system" fn device_wait_idle(device: vk::Device) -> vk::Result {
        let _guard = HostGuard::device(device);
        let candidate = device_dispatch(device).map(|dispatch| dispatch.device_idle);
        let Some(function) = original!(DEVICE_IDLE, candidate, vk::PFN_vkDeviceWaitIdle) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        unsafe { function(device) }
    }

    static GET_QUEUE: OnceLock<ExportHook<vk::PFN_vkGetDeviceQueue>> = OnceLock::new();
    unsafe extern "system" fn get_device_queue(
        device: vk::Device,
        family: u32,
        index: u32,
        queue: *mut vk::Queue,
    ) {
        let candidate = device_dispatch(device).map(|dispatch| dispatch.get_queue);
        if let Some(function) = original!(GET_QUEUE, candidate, vk::PFN_vkGetDeviceQueue) {
            unsafe { function(device, family, index, queue) };
            if !queue.is_null() && unsafe { *queue } != vk::Queue::null() {
                registry()
                    .queues
                    .insert(unsafe { *queue }.as_raw(), device.as_raw());
            }
        }
    }

    static GET_QUEUE2: OnceLock<ExportHook<GetQueue2Fn>> = OnceLock::new();
    unsafe extern "system" fn get_device_queue2(
        device: vk::Device,
        info: *const vk::DeviceQueueInfo2<'static>,
        queue: *mut vk::Queue,
    ) {
        let candidate = device_dispatch(device).and_then(|dispatch| dispatch.get_queue2);
        if let Some(function) = original!(GET_QUEUE2, candidate, GetQueue2Fn) {
            unsafe { function(device, info, queue) };
            if !queue.is_null() && unsafe { *queue } != vk::Queue::null() {
                registry()
                    .queues
                    .insert(unsafe { *queue }.as_raw(), device.as_raw());
            }
        }
    }

    static DESTROY: OnceLock<ExportHook<DestroyDeviceFn>> = OnceLock::new();
    unsafe extern "system" fn destroy_device(
        device: vk::Device,
        allocator: *const vk::AllocationCallbacks<'static>,
    ) {
        let _guard = HostGuard::device(device);
        let candidate = device_dispatch(device).map(|dispatch| dispatch.destroy);
        if let Some(function) = original!(DESTROY, candidate, DestroyDeviceFn) {
            unsafe { function(device, allocator) };
        }
        let mut registry = registry();
        registry.queues.retain(|_, owner| *owner != device.as_raw());
        registry.devices.remove(&device.as_raw());
        registry.host_devices.remove(&device.as_raw());
    }

    static GIPA: OnceLock<ExportHook<vk::PFN_vkGetInstanceProcAddr>> = OnceLock::new();
    static GDPA: OnceLock<ExportHook<vk::PFN_vkGetDeviceProcAddr>> = OnceLock::new();

    fn replacement(name: &CStr, native: vk::PFN_vkVoidFunction) -> vk::PFN_vkVoidFunction {
        native?;
        macro_rules! function {
            ($fn:ident, $ty:ty) => {
                Some(unsafe { std::mem::transmute::<$ty, unsafe extern "system" fn()>($fn as $ty) })
            };
        }
        match name.to_bytes() {
            b"vkQueueSubmit" => function!(queue_submit, SubmitFn),
            b"vkQueueSubmit2" => function!(queue_submit2, Submit2Fn),
            b"vkQueueSubmit2KHR" => function!(queue_submit2_khr, Submit2Fn),
            b"vkQueueBindSparse" => function!(queue_bind_sparse, BindSparseFn),
            b"vkQueuePresentKHR" => function!(queue_present, PresentFn),
            b"vkQueueWaitIdle" => function!(queue_wait_idle, vk::PFN_vkQueueWaitIdle),
            b"vkDeviceWaitIdle" => function!(device_wait_idle, vk::PFN_vkDeviceWaitIdle),
            b"vkGetDeviceQueue" => function!(get_device_queue, vk::PFN_vkGetDeviceQueue),
            b"vkGetDeviceQueue2" => function!(get_device_queue2, GetQueue2Fn),
            b"vkDestroyDevice" => function!(destroy_device, DestroyDeviceFn),
            b"vkGetDeviceProcAddr" => function!(get_device_proc_addr, vk::PFN_vkGetDeviceProcAddr),
            b"vkCreateDevice" => CREATE_DEVICE_WRAPPER
                .get()
                .map(|function| unsafe {
                    std::mem::transmute::<vk::PFN_vkCreateDevice, unsafe extern "system" fn()>(
                        *function,
                    )
                })
                .or(native),
            _ => native,
        }
    }

    unsafe extern "system" fn get_instance_proc_addr(
        instance: vk::Instance,
        name: *const c_char,
    ) -> vk::PFN_vkVoidFunction {
        let hook = GIPA.get()?;
        let native = unsafe { hook.detour.call(instance, name) };
        if name.is_null() {
            return native;
        }
        replacement(unsafe { CStr::from_ptr(name) }, native)
    }

    unsafe extern "system" fn get_device_proc_addr(
        device: vk::Device,
        name: *const c_char,
    ) -> vk::PFN_vkVoidFunction {
        let hook = GDPA.get()?;
        let native = unsafe { hook.detour.call(device, name) };
        if name.is_null() {
            return native;
        }
        replacement(unsafe { CStr::from_ptr(name) }, native)
    }

    pub(crate) fn register_created_device(device: vk::Device) -> Result<(), String> {
        let gdpa = GDPA
            .get()
            .ok_or("Vulkan device resolver hook unavailable")?;
        macro_rules! optional {
            ($name:literal, $ty:ty) => {{
                let pointer = unsafe {
                    gdpa.detour
                        .call(device, concat!($name, "\0").as_ptr().cast())
                };
                pointer.map(|pointer| unsafe {
                    std::mem::transmute::<unsafe extern "system" fn(), $ty>(pointer)
                })
            }};
        }
        macro_rules! required {
            ($name:literal, $ty:ty) => {
                optional!($name, $ty).ok_or(concat!("Vulkan device did not expose ", $name))?
            };
        }
        let dispatch = Dispatch {
            submit: required!("vkQueueSubmit", SubmitFn),
            submit2: optional!("vkQueueSubmit2", Submit2Fn),
            submit2_khr: optional!("vkQueueSubmit2KHR", Submit2Fn),
            bind_sparse: required!("vkQueueBindSparse", BindSparseFn),
            present: optional!("vkQueuePresentKHR", PresentFn),
            queue_idle: required!("vkQueueWaitIdle", vk::PFN_vkQueueWaitIdle),
            device_idle: required!("vkDeviceWaitIdle", vk::PFN_vkDeviceWaitIdle),
            get_queue: required!("vkGetDeviceQueue", vk::PFN_vkGetDeviceQueue),
            get_queue2: optional!("vkGetDeviceQueue2", GetQueue2Fn),
            destroy: required!("vkDestroyDevice", DestroyDeviceFn),
        };
        registry().devices.insert(
            device.as_raw(),
            DeviceRecord {
                dispatch,
                created_through_hook: true,
            },
        );
        Ok(())
    }

    pub(crate) fn ensure_queue_synchronization(
        device: vk::Device,
        queue: vk::Queue,
    ) -> Result<(), String> {
        if !INSTALLED.load(Ordering::Acquire) {
            return Err(
                "Vulkan queue host-synchronization hooks are not fully installed".to_string(),
            );
        }
        let registry = registry();
        let record = registry
            .devices
            .get(&device.as_raw())
            .ok_or("Vulkan device was not observed by the creation hook")?;
        if !record.created_through_hook
            || registry.queues.get(&queue.as_raw()) != Some(&device.as_raw())
        {
            return Err("Vulkan queue has no intercepted device/queue provenance".to_string());
        }
        Ok(())
    }

    pub(crate) fn install(
        library_name: &str,
        create_device: vk::PFN_vkCreateDevice,
    ) -> Result<(), String> {
        let _install = INSTALL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if INSTALLED.load(Ordering::Acquire) {
            return Ok(());
        }
        // Vulkan/volk retain the wrapper PFNs independently of GDExtension's
        // library handle. Keep their code and static state resident even if an
        // editor attempts to unload the extension. Native hooks require restart.
        if CALLBACK_LIBRARY.get().is_none() {
            let path = process_path::get_dylib_path()
                .ok_or("Cannot locate Vulkan hook callback library")?;
            let library = unsafe { libloading::Library::new(path) }
                .map_err(|error| format!("Cannot retain Vulkan hook callback library: {error}"))?;
            let _ = CALLBACK_LIBRARY.set(library);
        }
        if LIBRARY.get().is_none() {
            let library = unsafe { libloading::Library::new(library_name) }
                .map_err(|error| error.to_string())?;
            let _ = LIBRARY.set(library);
        }
        let library = LIBRARY.get().ok_or("Vulkan loader library unavailable")?;
        let _ = CREATE_DEVICE_WRAPPER.set(create_device);
        macro_rules! hook {
            ($static:ident, $name:literal, $function:ident, $ty:ty, $required:expr) => {{
                if $static.get().is_none() {
                    match unsafe { library.get::<$ty>(concat!($name, "\0").as_bytes()) } {
                        Ok(target) => {
                            let target = *target;
                            let detour =
                                unsafe { GenericDetour::<$ty>::new(target, $function as $ty) }
                                    .map_err(|error| format!("{} interception: {error}", $name))?;
                            let _ = $static.set(ExportHook {
                                address: target as usize,
                                detour,
                            });
                        }
                        Err(error) if $required => {
                            return Err(format!("{} export unavailable: {error}", $name));
                        }
                        Err(_) => {}
                    }
                }
                if let Some(hook) = $static.get() {
                    unsafe { hook.detour.enable() }
                        .map_err(|error| format!("{} hook enable: {error}", $name))?;
                }
            }};
        }
        // Mandatory 1.0 exports are patched as well as proc-address resolution:
        // pointers cached by volk before Core initialization still hit detours.
        hook!(SUBMIT, "vkQueueSubmit", queue_submit, SubmitFn, true);
        hook!(SUBMIT2, "vkQueueSubmit2", queue_submit2, Submit2Fn, false);
        hook!(
            SUBMIT2_KHR,
            "vkQueueSubmit2KHR",
            queue_submit2_khr,
            Submit2Fn,
            false
        );
        hook!(
            BIND_SPARSE,
            "vkQueueBindSparse",
            queue_bind_sparse,
            BindSparseFn,
            true
        );
        hook!(
            PRESENT,
            "vkQueuePresentKHR",
            queue_present,
            PresentFn,
            false
        );
        hook!(
            QUEUE_IDLE,
            "vkQueueWaitIdle",
            queue_wait_idle,
            vk::PFN_vkQueueWaitIdle,
            true
        );
        hook!(
            DEVICE_IDLE,
            "vkDeviceWaitIdle",
            device_wait_idle,
            vk::PFN_vkDeviceWaitIdle,
            true
        );
        hook!(
            GET_QUEUE,
            "vkGetDeviceQueue",
            get_device_queue,
            vk::PFN_vkGetDeviceQueue,
            true
        );
        hook!(
            GET_QUEUE2,
            "vkGetDeviceQueue2",
            get_device_queue2,
            GetQueue2Fn,
            false
        );
        hook!(
            DESTROY,
            "vkDestroyDevice",
            destroy_device,
            DestroyDeviceFn,
            true
        );
        hook!(
            GDPA,
            "vkGetDeviceProcAddr",
            get_device_proc_addr,
            vk::PFN_vkGetDeviceProcAddr,
            true
        );
        hook!(
            GIPA,
            "vkGetInstanceProcAddr",
            get_instance_proc_addr,
            vk::PFN_vkGetInstanceProcAddr,
            true
        );
        INSTALLED.store(true, Ordering::Release);
        eprintln!("[VulkanHook] Queue host synchronization installed (GIPA, GDPA, loader exports)");
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::AtomicUsize;

        #[test]
        fn resolver_replaces_every_host_synchronized_queue_operation() {
            unsafe extern "system" fn native() {}
            for name in [
                c"vkQueueSubmit",
                c"vkQueueSubmit2",
                c"vkQueueSubmit2KHR",
                c"vkQueueBindSparse",
                c"vkQueuePresentKHR",
                c"vkQueueWaitIdle",
                c"vkDeviceWaitIdle",
                c"vkGetDeviceQueue",
                c"vkGetDeviceQueue2",
                c"vkDestroyDevice",
                c"vkGetDeviceProcAddr",
            ] {
                let routed = replacement(name, Some(native));
                assert!(routed.is_some());
                assert_ne!(
                    routed.map(|pointer| pointer as usize),
                    Some(native as *const () as usize)
                );
                assert!(replacement(name, None).is_none());
            }
            assert_eq!(
                replacement(c"vkCreateBuffer", Some(native)).map(|pointer| pointer as usize),
                Some(native as *const () as usize)
            );
        }

        #[test]
        fn nested_dispatch_and_aliases_of_one_queue_are_serialized() {
            let active = AtomicUsize::new(0);
            let sync = Arc::new(DeviceHostSync::default());
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| {
                        for _ in 0..50 {
                            let _outer = HostGuard::acquire(Arc::clone(&sync), Some(17));
                            let _nested = HostGuard::acquire(Arc::clone(&sync), Some(17));
                            assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                            std::thread::yield_now();
                            assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
                        }
                    });
                }
            });
        }

        #[test]
        fn waiting_on_one_physical_queue_does_not_block_another_queue() {
            let sync = Arc::new(DeviceHostSync::default());
            std::thread::scope(|scope| {
                let waiting_queue = HostGuard::acquire(Arc::clone(&sync), Some(1));
                let (sender, receiver) = std::sync::mpsc::channel();
                let thread_sync = Arc::clone(&sync);
                scope.spawn(move || {
                    let _signal_queue = HostGuard::acquire(thread_sync, Some(2));
                    let _ = sender.send(());
                });
                let progressed = receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .is_ok();
                drop(waiting_queue);
                assert!(
                    progressed,
                    "A signal on queue 2 must progress while queue 1 waits"
                );
            });
        }

        #[test]
        fn device_idle_excludes_every_queue_then_releases_them() {
            let sync = Arc::new(DeviceHostSync::default());
            std::thread::scope(|scope| {
                let device_idle = HostGuard::acquire(Arc::clone(&sync), None);
                let (sender, receiver) = std::sync::mpsc::channel();
                let thread_sync = Arc::clone(&sync);
                scope.spawn(move || {
                    let _queue = HostGuard::acquire(thread_sync, Some(23));
                    let _ = sender.send(());
                });
                let blocked = receiver
                    .recv_timeout(std::time::Duration::from_millis(20))
                    .is_err();
                drop(device_idle);
                assert!(blocked);
                assert!(
                    receiver
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .is_ok()
                );
            });
        }

        #[test]
        fn unknown_queue_cannot_claim_interception_provenance() {
            assert!(ensure_queue_synchronization(vk::Device::null(), vk::Queue::null()).is_err());
        }
    }
}
