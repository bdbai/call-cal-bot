mod daka;

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

#[derive(Clone)]
pub struct Service {
    conn: Arc<Mutex<Connection>>,
}

impl Service {
    pub(super) fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }
}

pub fn init_service() -> Service {
    let mut conn = Connection::open("call-cal-bot.db").expect("Failed to open database");
    crate::migrations::runner()
        .run(&mut conn)
        .expect("db migration");
    Service::new(conn)
}
