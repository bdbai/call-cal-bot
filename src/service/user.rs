use crate::service::models::ServiceResponse;

impl super::Service {
    /// Find member id and password by qq_uin. Returns (id, password) on success.
    pub fn find_member_by_uin(&self, qq_uin: u32) -> Option<(i64, String)> {
        let conn_guard = self.conn.lock().unwrap();
        let mut stmt = match conn_guard
            .prepare_cached("SELECT id, password FROM bot_group_member WHERE qq_uin = ?1")
        {
            Ok(s) => s,
            Err(_) => return None,
        };
        let res: Result<(i64, String), _> = stmt.query_row([qq_uin], |row| {
            let id: i64 = row.get(0)?;
            let pw: String = row.get(1)?;
            Ok((id, pw))
        });
        drop(stmt);
        drop(conn_guard);
        res.ok()
    }

    pub fn set_password_for_member_id(
        &self,
        member_id: i64,
        hashed_password: &str,
    ) -> Result<(), ServiceResponse> {
        // delegate to service's update_password_by_id
        match self.update_password_by_id(member_id, hashed_password) {
            Ok(_) => Ok(()),
            Err(e) => Err(ServiceResponse::err(e)),
        }
    }
}
