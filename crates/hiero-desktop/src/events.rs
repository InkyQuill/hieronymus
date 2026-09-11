//! A level-triggered socket wakes GTK without a polling thread or event queue.
use crate::menu::MenuId;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Sender {
    socket: Arc<UnixStream>,
    action: Arc<Mutex<Option<MenuId>>>,
}
pub struct Receiver {
    pub socket: UnixStream,
    action: Arc<Mutex<Option<MenuId>>>,
}
pub fn channel() -> std::io::Result<(Sender, Receiver)> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    writer.set_nonblocking(true)?;
    let action = Arc::new(Mutex::new(None));
    Ok((
        Sender {
            socket: Arc::new(writer),
            action: action.clone(),
        },
        Receiver {
            socket: reader,
            action,
        },
    ))
}
impl Sender {
    pub fn wake(&self) {
        let _ = (&*self.socket).write(&[1]);
    }
    pub fn submit(&self, id: MenuId) {
        let mut pending = self
            .action
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.is_none() {
            *pending = Some(id);
        }
        drop(pending);
        self.wake();
    }
}
impl Receiver {
    pub fn drain(&self) -> Option<MenuId> {
        let mut bytes = [0; 256];
        while (&self.socket).read(&mut bytes).is_ok_and(|n| n > 0) {}
        self.action
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}
