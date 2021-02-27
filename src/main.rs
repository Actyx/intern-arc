use std::{slice::from_raw_parts, sync::Arc};

fn main() {
    let x: Arc<str> = "hello".into();
    let s = unsafe { from_raw_parts(&x as *const _ as *const u8, 16) };
    println!("{:?}", s);
}
