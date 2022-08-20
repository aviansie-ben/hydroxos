use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Debug;
use core::ptr;

use dyn_dyn::{dyn_dyn_base, dyn_dyn_cast, dyn_dyn_impl, DynDynBase, DynDynTable, GetDynDynTable};

use crate::io::dev::hub::{DeviceHub, VirtualDeviceHub};
use crate::log;
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;
use crate::sync::UninterruptibleSpinlock;
use crate::util::SharedUnsafeCell;

pub mod hub;

pub type DeviceRef<T> = Arc<DeviceLock<T>>;
pub type DeviceWeak<T> = Weak<DeviceLock<T>>;

#[dyn_dyn_base]
pub trait Device: Send + Sync + Debug + 'static {
    fn type_name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }

    unsafe fn on_connected(&mut self, _own_ref: &DeviceRef<Self>)
    where
        Self: Sized
    {
    }

    unsafe fn on_disconnected(&mut self) {}
}

#[derive(Debug)]
struct DummyDevice {}

#[dyn_dyn_impl]
impl Device for DummyDevice {}

#[derive(Debug)]
pub struct DeviceLock<T: ?Sized> {
    parent: DeviceWeak<dyn Device>,
    name: Box<str>,
    dev_table: DynDynTable,
    dev: UninterruptibleSpinlock<T>
}

impl<T: Device> DeviceLock<T> {
    pub fn new(name: Box<str>, dev: T) -> DeviceLock<T> {
        DeviceLock {
            parent: <DeviceWeak<DummyDevice>>::new(),
            name,
            dev_table: GetDynDynTable::<dyn Device>::get_dyn_dyn_table(&&dev),
            dev: UninterruptibleSpinlock::new(dev)
        }
    }

    pub fn connect(mut self, parent: DeviceWeak<dyn Device>) -> DeviceRef<T> {
        self.parent = parent;

        let dev = Arc::new(self);

        unsafe {
            dev.lock().on_connected(&dev);
        }

        dev
    }
}

impl<T: ?Sized> DeviceLock<T> {
    pub fn parent_dev(&self) -> &DeviceWeak<dyn Device> {
        &self.parent
    }

    pub fn name(&self) -> &str {
        &*self.name
    }

    pub fn full_name(&self) -> impl fmt::Display + '_ {
        DeviceFullName(self)
    }

    pub fn lock(&self) -> UninterruptibleSpinlockGuard<T> {
        self.dev.lock()
    }

    pub fn try_lock(&self) -> Option<UninterruptibleSpinlockGuard<T>> {
        self.dev.try_lock()
    }
}

unsafe impl DynDynBase for DeviceLock<dyn Device> {
    fn get_dyn_dyn_table(&self) -> DynDynTable {
        self.dev_table
    }
}

struct DeviceFullName<'a, T: ?Sized>(&'a DeviceLock<T>);

impl<'a, T: ?Sized> fmt::Display for DeviceFullName<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let DeviceFullName(dev) = *self;

        if let Some(parent) = dev.parent.upgrade() {
            write!(f, "{}::", DeviceFullName(&*parent))?;
        } else if !ptr::eq(
            &**device_root() as *const DeviceLock<dyn Device> as *const DeviceLock<()>,
            dev as *const DeviceLock<T> as *const DeviceLock<()>
        ) {
            write!(f, "(???)::")?;
        }

        write!(f, "{}", dev.name)
    }
}

static DEVICE_ROOT: SharedUnsafeCell<Option<DeviceRef<VirtualDeviceHub>>> = SharedUnsafeCell::new(None);

pub(crate) unsafe fn init_device_root() {
    debug_assert!((*DEVICE_ROOT.get()).is_none());

    let device_root = Arc::new(DeviceLock::new(Box::from("root"), VirtualDeviceHub::new()));

    device_root.lock().on_connected(&device_root);
    *DEVICE_ROOT.get() = Some(device_root);
}

pub fn device_root() -> &'static DeviceRef<VirtualDeviceHub> {
    unsafe { (*DEVICE_ROOT.get()).as_ref().unwrap() }
}

pub fn log_device_tree() {
    let mut line = String::new();

    fn dump_dev(line: &mut String, dev: &DeviceRef<dyn Device>, indent: u32) {
        use core::fmt::Write;

        line.clear();

        for _ in 0..indent {
            write!(line, "  ").unwrap();
        }

        write!(line, "{}", dev.name()).unwrap();

        let children = if let Some(lock) = dev.try_lock() {
            let type_name = {
                let type_name = lock.type_name();
                let short_idx = type_name.rfind("::").map_or(0, |i| i + 2);

                &type_name[short_idx..]
            };

            let impls = GetDynDynTable::<dyn Device>::get_dyn_dyn_table(&&*lock);

            let children: Option<Vec<_>> = if let Ok(hub) = dyn_dyn_cast!(Device => DeviceHub, &lock) {
                Some(hub.children().iter().cloned().collect())
            } else {
                None
            };

            drop(lock);

            write!(line, ": {}", type_name).unwrap();

            if !impls.into_slice().is_empty() {
                let mut need_comma = false;

                write!(line, " [ ").unwrap();
                for entry in impls {
                    if need_comma {
                        write!(line, ", ").unwrap();
                    } else {
                        need_comma = true;
                    }

                    let name = entry.type_name();
                    let short_idx = name.rfind("::").map_or(0, |i| i + 2);

                    write!(line, "{}", &name[short_idx..]).unwrap();
                }

                write!(line, " ]").unwrap();
            }

            children
        } else {
            write!(line, " (locked)").unwrap();
            None
        };

        log!(Debug, "dev", "{}", line);

        if let Some(children) = children {
            for child in children {
                dump_dev(line, &child, indent + 1);
            }
        }
    }

    dump_dev(&mut line, &(device_root().clone() as DeviceRef<dyn Device>), 0);
}
