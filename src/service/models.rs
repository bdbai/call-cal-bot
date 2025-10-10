/// Internal business model for a group member. Handlers convert mania's
/// `BotGroupMember` into this struct before calling service methods.
#[derive(Debug, Clone)]
pub struct GroupMember {
    pub uid: String,
    pub uin: u32,
    pub member_name: Option<String>,
    pub member_card: Option<String>,
}

impl GroupMember {
    pub fn nickname(&self) -> &str {
        self.member_name.as_deref().unwrap_or("")
    }
    pub fn group_nickname(&self) -> &str {
        self.member_card.as_deref().unwrap_or(self.nickname())
    }
}

/// Response from service methods. `message` is the human-readable content;
/// `ok` is true when the operation was successful (e.g. DB insert succeeded),
/// false otherwise.
#[derive(Debug, Clone)]
pub struct ServiceResponse {
    pub message: String,
    pub ok: bool,
}

impl ServiceResponse {
    pub fn ok<M: Into<String>>(msg: M) -> Self {
        ServiceResponse {
            message: msg.into(),
            ok: true,
        }
    }

    pub fn err<M: Into<String>>(msg: M) -> Self {
        ServiceResponse {
            message: msg.into(),
            ok: false,
        }
    }
}
