use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Debug;
use core::marker::Unsize;
use core::ops::{CoerceUnsized, Deref};
use core::ptr;
use core::ptr::Pointee;

use dyn_dyn::{dyn_dyn_base, dyn_dyn_cast, dyn_dyn_impl, DowncastUnchecked, DynDynBase, DynDynTable, GetDynDynTable};

use crate::io::dev::hub::{DeviceHub, DeviceHubExt, DeviceHubLockedError, VirtualDeviceHub};
use crate::log;
use crate::sync::future::FutureWriter;
use crate::sync::{Future, UninterruptibleSpinlock};
use crate::util::OneShotManualInit;

pub mod hub;
pub mod kbd;

pub struct DeviceRef<T: ?Sized>(Arc<DeviceNode<T>>);

impl<T> DeviceRef<T> {
    pub fn new(val: DeviceNode<T>) -> DeviceRef<T> {
        DeviceRef(Arc::new(val))
    }
}

impl<T: ?Sized> DeviceRef<T> {
    pub fn downgrade(dev: &Self) -> DeviceWeak<T> {
        DeviceWeak(Arc::downgrade(&dev.0))
    }
}

impl<T: ?Sized> Clone for DeviceRef<T> {
    fn clone(&self) -> Self {
        DeviceRef(self.0.clone())
    }
}

impl<T: ?Sized> Deref for DeviceRef<T> {
    type Target = DeviceNode<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<DeviceRef<U>> for DeviceRef<T> {}

impl<T: ?Sized> Debug for DeviceRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceRef({})", self.full_name())
    }
}

unsafe impl<B: ?Sized + DynDynBase, T: ?Sized + Unsize<B>> GetDynDynTable<B> for DeviceRef<T> {
    type DynTarget = T;

    fn get_dyn_dyn_table(&self) -> DynDynTable {
        B::get_dyn_dyn_table(self.dev())
    }
}

impl<'a, T: ?Sized + 'a> DowncastUnchecked<'a> for DeviceRef<T> {
    type DowncastResult<D: ?Sized + 'a> = DeviceRef<D>;

    unsafe fn downcast_unchecked<D: ?Sized + Pointee>(self, metadata: <D as Pointee>::Metadata) -> DeviceRef<D> {
        DeviceRef(DowncastUnchecked::downcast_unchecked::<DeviceNode<D>>(self.0, metadata))
    }
}

pub struct DeviceWeak<T: ?Sized>(Weak<DeviceNode<T>>);

impl<T> DeviceWeak<T> {
    pub fn new() -> Self {
        Self(Weak::new())
    }
}

impl<T: ?Sized> DeviceWeak<T> {
    pub fn strong_count(&self) -> usize {
        self.0.strong_count()
    }

    pub fn upgrade(&self) -> Option<DeviceRef<T>> {
        self.0.upgrade().map(DeviceRef)
    }
}

impl<T: ?Sized> Clone for DeviceWeak<T> {
    fn clone(&self) -> Self {
        DeviceWeak(self.0.clone())
    }
}

impl<T: ?Sized> Debug for DeviceWeak<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.upgrade() {
            Some(val) => write!(f, "DeviceWeak({})", val.full_name()),
            None => write!(f, "DeviceWeak(<freed>)")
        }
    }
}

impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<DeviceWeak<U>> for DeviceWeak<T> {}

#[derive(Debug)]
pub struct DeviceNotFoundError;

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

        let dev = DeviceRef::new(self);

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
        &self.name
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
        let root_dev = &**device_root() as *const _ as *const ();

        if let Some(parent) = dev.parent.upgrade() {
            if ptr::eq(root_dev, &*parent as *const _ as *const ()) {
                write!(f, "::{}", dev.name)?;
            } else {
                write!(f, "{}::{}", DeviceFullName(&*parent), dev.name)?;
            }
        } else if !ptr::eq(root_dev, dev as *const _ as *const ()) {
            write!(f, "(???)::{}", dev.name)?;
        } else {
            write!(f, "{}", dev.name)?;
        }

        Ok(())
    }
}

static DEVICE_ROOT: OneShotManualInit<DeviceRef<VirtualDeviceHub>> = OneShotManualInit::uninit();

pub(crate) unsafe fn init_device_root() {
    let device_root = DeviceRef::new(DeviceNode::new(Box::from("(root)"), VirtualDeviceHub::new()));

    device_root.dev.on_connected(&device_root);
    DEVICE_ROOT.set(device_root);
}

pub fn device_root() -> &'static DeviceRef<VirtualDeviceHub> {
    DEVICE_ROOT.get()
}

pub fn get_device_by_name(mut name: &str) -> Result<DeviceRef<dyn Device>, DeviceNotFoundError> {
    let mut hub: DeviceRef<dyn Device> = device_root().clone();

    while let Some(end_part) = name.find("::") {
        let name_part = &name[..end_part];
        name = &name[end_part + 2..];

        let hub_dev = if let Ok(hub) = dyn_dyn_cast!(move Device => DeviceHub, hub.dev()) {
            hub
        } else {
            return Err(DeviceNotFoundError);
        };

        if let Some(dev) = hub_dev.find_child(name_part) {
            hub = dev;
        }
    }

    let hub_dev = if let Ok(hub) = dyn_dyn_cast!(move Device => DeviceHub, hub.dev()) {
        hub
    } else {
        return Err(DeviceNotFoundError);
    };

    if let Some(dev) = hub_dev.find_child(name) {
        Ok(dev)
    } else {
        Err(DeviceNotFoundError)
    }
}

fn print_device_tree_internal<E>(root: &DeviceRef<dyn Device>, mut f: impl FnMut(&str) -> Result<(), E>) -> Result<(), E> {
    let mut line = String::new();

    fn dump_dev<E>(
        f: &mut impl FnMut(&str) -> Result<(), E>,
        line: &mut String,
        dev: &DeviceRef<dyn Device>,
        indent: u32
    ) -> Result<(), E> {
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

            let result = hub.try_for_children(&mut |c| {
                children.push(c.clone());
                true
            });

            Some(match result {
                Ok(_) => Ok(children),
                Err(e) => Err(e)
            })
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

        f(line)?;

        match children {
            Some(Ok(mut children)) => {
                children.sort_by(|a, b| a.name().cmp(b.name()));
                for child in children {
                    dump_dev(f, line, &child, indent + 1)?;
                }
            },
            Some(Err(_)) => {
                line.clear();
                for _ in 0..(indent + 1) {
                    write!(line, "  ").unwrap();
                }

                write!(line, "(hub locked)").unwrap();

                f(line)?;
            },
            None => {}
        }

        Ok(())
    }

    dump_dev(&mut f, &mut line, root, 0)
}

pub fn print_device_tree<T: fmt::Write>(w: &mut T, root: &DeviceRef<dyn Device>) -> Result<(), fmt::Error> {
    print_device_tree_internal(root, |line| writeln!(w, "{}", line))
}

pub fn log_device_tree() {
    print_device_tree_internal(&(device_root().clone() as DeviceRef<dyn Device>), |line| {
        log!(Debug, "dev", "{}", line);
        Ok(()) as Result<(), ()>
    })
    .unwrap();
}
