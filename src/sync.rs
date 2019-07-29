use crossbeam_channel::{unbounded, Sender};
use lazy_static::lazy_static;
use parking_lot::Mutex;
use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::transmute;
use std::thread;
use weechat::Weechat;

/// Created upon sync initialization, must not be dropped while the plugin is running
pub struct SyncHandle(weechat::TimerHook<()>);

enum Job {
    Nonblocking(Box<dyn FnOnce(&Weechat) + Send>),
    Blocking(
        Box<dyn FnOnce(&Weechat) -> (Box<dyn Any + Send>) + Send>,
        Sender<Box<dyn Any + Send>>,
    ),
}

lazy_static! {
    static ref JOB_QUEUE: Mutex<RefCell<VecDeque<Job>>> = Mutex::new(RefCell::new(VecDeque::new()));
}

static mut MAIN_THREAD: Option<thread::ThreadId> = None;

/// Initialize thread synchronization, this function must be called on the main thread
pub fn init(weechat: &weechat::Weechat) -> SyncHandle {
    unsafe {
        MAIN_THREAD = Some(thread::current().id());
    }

    // TODO: Dynamic delay
    SyncHandle(weechat.hook_timer(25, 0, 0, |_, _| tick(), None))
}

pub fn on_main<F: 'static + FnOnce(&Weechat) + Send>(cb: F) {
    if std::thread::current().id() == unsafe { MAIN_THREAD.unwrap() } {
        // already on the main thread, run closure now
        cb(unsafe { &crate::__PLUGIN.as_ref().unwrap().weechat });
    } else {
        // queue closure for later
        JOB_QUEUE
            .lock()
            .borrow_mut()
            .push_back(Job::Nonblocking(Box::new(cb)));
    }
}

pub fn on_main_blocking<R: Send, F: FnOnce(&Weechat) -> R + Send, ER: 'static + Send>(cb: F) -> ER {
    let cb = unsafe {
        // This should be ok because the lifetime does not actually
        // have to be valid for 'static, just until the function returns
        transmute::<
            Box<dyn FnOnce(&Weechat) -> R + Send>,
            Box<dyn 'static + FnOnce(&Weechat) -> ER + Send>,
        >(Box::new(cb))
    };

    if std::thread::current().id() == unsafe { MAIN_THREAD.unwrap() } {
        cb(unsafe { &crate::__PLUGIN.as_ref().unwrap().weechat })
    } else {
        let (tx, rx) = unbounded();
        let job = Job::Blocking(Box::new(move |data| Box::new(cb(data))), tx);
        JOB_QUEUE.lock().borrow_mut().push_back(job);

        let rcv: Box<dyn Any + Send> = rx.recv().expect("rx can't fail");
        *rcv.downcast::<ER>().expect("downcast can't fail")
    }
}

fn tick() {
    match JOB_QUEUE.lock().borrow_mut().pop_front() {
        Some(Job::Nonblocking(cb)) => {
            (cb)(unsafe { &crate::__PLUGIN.as_ref().unwrap().weechat });
        }
        Some(Job::Blocking(cb, tx)) => {
            let result = (cb)(unsafe { &crate::__PLUGIN.as_ref().unwrap().weechat });
            let _ = tx.send(result);
        }
        None => {}
    }
}

