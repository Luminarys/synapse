use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

pub struct AView<T: 'static> {
    /// This is actually the Arc
    ptr: *const u8,
    val: &'static T,
}

impl<T> AView<T> {
    pub fn new<'a, U: 'static, F: FnOnce(&'a U) -> &'a T>(arc: &'a Arc<U>, f: F) -> AView<T> {
        let val_ptr = f(&*arc) as *const T;
        let val = unsafe { val_ptr.as_ref().unwrap() };
        let ptr = Arc::into_raw(arc.clone()) as *const u8;
        AView { val, ptr }
    }

    pub fn value(v: T) -> AView<T> {
        let a = Arc::new(v);
        AView::new(&a, |r| r)
    }
}

impl<T> AsRef<T> for AView<T> {
    fn as_ref(&self) -> &T {
        self.val
    }
}

impl<T> Deref for AView<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.val
    }
}

impl<T: fmt::Debug> fmt::Debug for AView<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AView: {:?}", self.as_ref())
    }
}

impl<T> Clone for AView<T> {
    fn clone(&self) -> AView<T> {
        unsafe {
            let a = Arc::from_raw(self.ptr);
            let c = AView {
                ptr: Arc::into_raw(a.clone()),
                val: self.val,
            };
            Arc::into_raw(a);
            c
        }
    }
}

unsafe impl<T: Send + Sync> Send for AView<T> {}
unsafe impl<T: Send + Sync> Sync for AView<T> {}

impl<T> Drop for AView<T> {
    fn drop(&mut self) {
        unsafe {
            drop(Arc::from_raw(self.ptr));
        }
    }
}
