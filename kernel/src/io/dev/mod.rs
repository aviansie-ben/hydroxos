use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Debug;
use core::ptr;

use dyn_dyn::{dyn_dyn_base, dyn_dyn_cast, dyn_dyn_impl, DynDynBase, DynDynTable, GetDynDynTable};

use crate::io::dev::hub::{DeviceHub, DeviceHubLockedError, VirtualDeviceHub};
use crate::log;
use crate::sync::future::FutureWriter;
use crate::sync::{Future, UninterruptibleSpinlock};
use crate::util::SharedUnsafeCell;

pub mod hub;

pub type DeviceRef<T> = Arc<DeviceNode<T>>;
pub type DeviceWeak<T> = Weak<DeviceNode<T>>;

#[dyn_dyn_base]
pub trait Device: Send + Sync + Debug + 'static {
    fn type_name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }

    unsafe fn on_connected(&self, _own_ref: &DeviceRef<Self>)
    where
        Self: Sized
    {
    }

    unsafe fn on_disconnected(&self) {}
}

#[derive(Debug)]
struct DummyDevice {}

#[dyn_dyn_impl]
impl Device for DummyDevice {}

#[derive(Debug)]
pub struct DeviceNode<T: ?Sized> {
    parent: DeviceWeak<dyn Device>,
    name: Box<str>,
    disconnect_event: UninterruptibleSpinlock<Option<FutureWriter<()>>>,
    dev: T
}

impl<T: Device> DeviceNode<T> {
    pub fn new(name: Box<str>, dev: T) -> DeviceNode<T> {
        DeviceNode {
            parent: <DeviceWeak<DummyDevice>>::new(),
            name,
            disconnect_event: UninterruptibleSpinlock::new(None),
            dev
        }
    }

    pub fn connect(mut self, parent: DeviceWeak<dyn Device>) -> DeviceRef<T> {
        self.parent = parent;
        self.disconnect_event = UninterruptibleSpinlock::new(Some(FutureWriter::new()));

        let dev = Arc::new(self);

        unsafe {
            dev.dev.on_connected(&dev);
        }

        dev
    }
}

impl<T: Device + ?Sized> DeviceNode<T> {
    pub fn disconnect(&self) {
        if let Some(disconnect_event) = self.disconnect_event.lock().take() {
            disconnect_event.finish(());
        } else {
            panic!("Cannot disconnect an already disconnected device");
        }

        unsafe {
            self.dev.on_disconnected();
        }
    }
}

impl<T: ?Sized> DeviceNode<T> {
    pub fn parent_dev(&self) -> &DeviceWeak<dyn Device> {
        &self.parent
    }

    pub fn name(&self) -> &str {
        &*self.name
    }

    pub fn full_name(&self) -> impl fmt::Display + '_ {
        DeviceFullName(self)
    }

    pub fn dev(&self) -> &T {
        &self.dev
    }

    pub fn when_disconnected(&self) -> Future<()> {
        self.disconnect_event
            .lock()
            .as_ref()
            .map_or_else(|| Future::done(()), |w| w.as_future())
    }
}

unsafe impl DynDynBase for DeviceNode<dyn Device> {
    fn get_dyn_dyn_table(&self) -> DynDynTable {
        GetDynDynTable::<dyn Device>::get_dyn_dyn_table(&&self.dev)
    }
}

struct DeviceFullName<'a, T: ?Sized>(&'a DeviceNode<T>);

impl<'a, T: ?Sized> fmt::Display for DeviceFullName<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let DeviceFullName(dev) = *self;

        if let Some(parent) = dev.parent.upgrade() {
            write!(f, "{}::", DeviceFullName(&*parent))?;
        } else if !ptr::eq(
            &**device_root() as *const DeviceNode<dyn Device> as *const DeviceNode<()>,
            dev as *const DeviceNode<T> as *const DeviceNode<()>
        ) {
            write!(f, "(???)::")?;
        }

        write!(f, "{}", dev.name)
    }
}

static DEVICE_ROOT: SharedUnsafeCell<Option<DeviceRef<VirtualDeviceHub>>> = SharedUnsafeCell::new(None);

pub(crate) unsafe fn init_device_root() {
    debug_assert!((*DEVICE_ROOT.get()).is_none());

    let device_root = Arc::new(DeviceNode::new(Box::from("root"), VirtualDeviceHub::new()));

    device_root.dev.on_connected(&device_root);
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

        let type_name = {
            let type_name = dev.dev().type_name();
            let short_idx = type_name.rfind("::").map_or(0, |i| i + 2);

            &type_name[short_idx..]
        };

        let impls = GetDynDynTable::<dyn Device>::get_dyn_dyn_table(&dev.dev());

        let children: Option<Result<Vec<_>, DeviceHubLockedError>> = if let Ok(hub) = dyn_dyn_cast!(Device => DeviceHub, dev.dev()) {
            let mut children = vec![];

            Some(
                match hub.try_for_children(&mut |c| {
                    children.push(c.clone());
                    true
                }) {
                    Ok(_) => Ok(children),
                    Err(e) => Err(e)
                }
            )
        } else {
            None
        };

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

        log!(Debug, "dev", "{}", line);

        match children {
            Some(Ok(children)) => {
                for child in children {
                    dump_dev(line, &child, indent + 1);
                }
            },
            Some(Err(_)) => {
                line.clear();
                for _ in 0..(indent + 1) {
                    write!(line, "  ").unwrap();
                }

                write!(line, "(hub locked)").unwrap();

                log!(Debug, "dev", "{}", line);
            },
            None => {}
        }
    }

    dump_dev(&mut line, &(device_root().clone() as DeviceRef<dyn Device>), 0);
}
