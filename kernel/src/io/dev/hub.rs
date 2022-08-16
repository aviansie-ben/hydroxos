use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Debug;
use core::ptr;

use dyn_dyn::dyn_dyn_derived;
use itertools::Itertools;

use crate::io::dev::{Device, DeviceLock, DeviceRef, DeviceWeak};

pub trait DeviceHub: Device {
    fn children(&self) -> &[DeviceRef<dyn Device>];
}

#[derive(Debug)]
pub struct VirtualDeviceHub {
    own_ref: DeviceWeak<VirtualDeviceHub>,
    children: Vec<DeviceRef<dyn Device>>
}

impl VirtualDeviceHub {
    pub fn new() -> VirtualDeviceHub {
        VirtualDeviceHub {
            own_ref: Weak::new(),
            children: vec![]
        }
    }

    fn assert_connected(&self) {
        if self.own_ref.strong_count() == 0 {
            panic!("Cannot operate on disconnected VirtualDeviceHub");
        }
    }

    pub fn add_device<T: Device>(&mut self, dev: DeviceLock<T>) -> DeviceRef<T> {
        self.assert_connected();

        let dev = dev.connect(self.own_ref.clone());

        self.children.push(dev.clone());
        dev
    }

    pub fn remove_device(&mut self, dev: &DeviceRef<dyn Device>) {
        let dev = &**dev;
        if let Some((idx, _)) = self.children.iter().find_position(|&child| ptr::eq(&**child, dev)) {
            self.children.remove(idx);
        } else {
            panic!("Attempt to remove device from VirtualDeviceHub that it's not connected to");
        }
    }
}

#[dyn_dyn_derived(DeviceHub)]
impl Device for VirtualDeviceHub {
    unsafe fn on_connected(&mut self, own_ref: &DeviceRef<Self>)
    where
        Self: Sized
    {
        self.own_ref = Arc::downgrade(own_ref);
    }

    unsafe fn on_disconnected(&mut self) {
        self.own_ref = Weak::new();
        for child in self.children.drain(..) {
            child.dev.lock().on_disconnected();
        }
    }
}

impl DeviceHub for VirtualDeviceHub {
    fn children(&self) -> &[DeviceRef<dyn Device>] {
        &self.children
    }
}
