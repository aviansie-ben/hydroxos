use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Debug;
use core::ptr;

use dyn_dyn::dyn_dyn_impl;
use itertools::Itertools;

use crate::io::dev::{Device, DeviceNode, DeviceRef, DeviceWeak};
use crate::sync::UninterruptibleSpinlock;

#[derive(Debug)]
pub struct DeviceHubLockedError;

pub trait DeviceHub: Device {
    fn for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> bool;
    fn try_for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> Result<bool, DeviceHubLockedError> {
        Ok(self.for_children(f))
    }
}

pub trait DeviceHubExt: DeviceHub {
    fn collect_children(&self, children: &mut Vec<DeviceRef<dyn Device>>);
    fn children(&self) -> Vec<DeviceRef<dyn Device>>;
    fn find_child(&self, name: &str) -> Option<DeviceRef<dyn Device>>;
}

impl<T: DeviceHub + ?Sized> DeviceHubExt for T {
    fn collect_children(&self, children: &mut Vec<DeviceRef<dyn Device>>) {
        self.for_children(&mut |c| {
            children.push(c.clone());
            true
        });
    }

    fn children(&self) -> Vec<DeviceRef<dyn Device>> {
        let mut children = vec![];
        self.collect_children(&mut children);
        children
    }

    fn find_child(&self, name: &str) -> Option<DeviceRef<dyn Device>> {
        let mut dev = None;

        self.for_children(&mut |child| {
            if child.name() == name {
                dev = Some(child.clone());
                false
            } else {
                true
            }
        });

        dev
    }
}

#[derive(Debug)]
struct VirtualDeviceHubInternals {
    own_ref: DeviceWeak<VirtualDeviceHub>,
    children: Vec<DeviceRef<dyn Device>>,
}

impl VirtualDeviceHubInternals {
    pub fn new() -> VirtualDeviceHubInternals {
        VirtualDeviceHubInternals {
            own_ref: DeviceWeak::new(),
            children: vec![],
        }
    }

    fn assert_connected(&self) {
        if self.own_ref.strong_count() == 0 {
            panic!("Cannot operate on disconnected VirtualDeviceHub");
        }
    }

    fn add_device<T: Device>(&mut self, dev: DeviceNode<T>) -> DeviceRef<T> {
        self.assert_connected();

        let dev = dev.connect(self.own_ref.clone());

        self.children.push(dev.clone());
        dev
    }

    fn remove_device(&mut self, dev: &DeviceRef<dyn Device>) {
        let dev = &**dev;
        if let Some((idx, _)) = self.children.iter().find_position(|&child| ptr::eq(&**child, dev)) {
            self.children.remove(idx);
        } else {
            panic!("Attempt to remove device from VirtualDeviceHub that it's not connected to");
        }
    }

    unsafe fn on_connected(&mut self, own_ref: &DeviceRef<VirtualDeviceHub>) {
        self.own_ref = DeviceRef::downgrade(own_ref);
    }

    unsafe fn on_disconnected(&mut self) {
        self.own_ref = DeviceWeak::new();
        for child in self.children.drain(..) {
            child.disconnect();
        }
    }

    fn for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> bool {
        for child in self.children.iter() {
            if !f(child) {
                return false;
            }
        }

        true
    }
}

#[derive(Debug)]
pub struct VirtualDeviceHub {
    internal: UninterruptibleSpinlock<VirtualDeviceHubInternals>,
}

impl VirtualDeviceHub {
    pub fn new() -> VirtualDeviceHub {
        VirtualDeviceHub {
            internal: UninterruptibleSpinlock::new(VirtualDeviceHubInternals::new()),
        }
    }

    pub fn add_device<T: Device>(&self, dev: DeviceNode<T>) -> DeviceRef<T> {
        self.internal.lock().add_device(dev)
    }

    pub fn remove_device<T: Device>(&self, dev: &DeviceRef<dyn Device>) {
        self.internal.lock().remove_device(dev)
    }
}

#[dyn_dyn_impl(DeviceHub)]
impl Device for VirtualDeviceHub {
    unsafe fn on_connected(&self, own_ref: &DeviceRef<VirtualDeviceHub>) {
        self.internal.lock().on_connected(own_ref);
    }

    unsafe fn on_disconnected(&self) {
        self.internal.lock().on_disconnected();
    }
}

impl DeviceHub for VirtualDeviceHub {
    fn for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> bool {
        self.internal.lock().for_children(f)
    }

    fn try_for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> Result<bool, DeviceHubLockedError> {
        match self.internal.try_lock() {
            Some(lock) => Ok(lock.for_children(f)),
            None => Err(DeviceHubLockedError),
        }
    }
}
