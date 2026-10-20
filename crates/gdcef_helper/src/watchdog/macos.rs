pub(super) fn prepare_parent_watchdog(parent_pid: u32) -> Result<Option<usize>, ()> {
    if !is_parent_alive(parent_pid, 0) {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    }

    let queue = unsafe { libc::kqueue() };
    if queue < 0 {
        eprintln!("Failed to create gdcef parent watchdog kqueue");
        return Err(());
    }

    let event = libc::kevent {
        ident: parent_pid as libc::uintptr_t,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    let registered =
        unsafe { libc::kevent(queue, &event, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if registered < 0 || !is_parent_alive(parent_pid, 0) {
        unsafe { libc::close(queue) };
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    }

    Ok(Some(queue as usize))
}

pub(super) fn is_parent_alive(parent_pid: u32, _watch: usize) -> bool {
    unsafe { libc::getppid() as u32 == parent_pid }
}

pub(super) fn wait_for_parent(_parent_pid: u32, watch: usize) {
    let queue = watch as libc::c_int;
    let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
    unsafe {
        libc::kevent(
            queue,
            std::ptr::null(),
            0,
            event.as_mut_ptr(),
            1,
            std::ptr::null(),
        );
        libc::close(queue);
    }
}
