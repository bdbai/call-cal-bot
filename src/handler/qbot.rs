use std::fs;

use mania::entity::bot_group_member::BotGroupMember;
use mania::event::group::GroupEvent;
use mania::event::group::group_message::GroupMessageEvent;
use mania::message::builder::MessageChainBuilder;
use mania::message::chain::{GroupMessageUniqueElem, MessageChain, MessageType};
use mania::message::entity::{Entity, Mention};
use mania::{Client, ClientConfig, DeviceInfo, KeyStore};
use tracing::debug;

use crate::service::Service;
use crate::service::models::GroupMember;

fn bot_member_to_group_member(b: &BotGroupMember) -> GroupMember {
    GroupMember {
        uid: b.uid.to_string(),
        uin: b.uin,
        member_name: b.member_name.clone(),
        member_card: b.member_card.clone(),
    }
}

fn handle_group_msg(svc: &Service, ev: &GroupMessageEvent) -> Option<MessageChain> {
    let MessageType::Group(GroupMessageUniqueElem {
        group_uin,
        group_member_info: Some(group_member_info),
    }) = &ev.chain.typ
    else {
        return None;
    };
    debug!("group_member_info: {:?}", group_member_info);

    let text_entities = ev.chain.entities.iter().filter_map(|e| {
        if let Entity::Text(te) = e {
            Some(te.text.trim())
        } else {
            None
        }
    });
    let first_text = text_entities
        .filter(|te| !te.is_empty())
        .next()
        .unwrap_or_default();
    let (command, args) = first_text.split_once(' ').unwrap_or((first_text, ""));

    let gm = bot_member_to_group_member(group_member_info);
    match command {
        "/打卡" => {
            debug!("Handling 我没打卡 command for user {}", gm.uin);
            match svc.upsert_member(&gm) {
                Ok(user_id) => {
                    let res = svc.handle_打卡(user_id, args);
                    tracing::debug!("Service handle_打卡 ok={} message={}", res.ok, res.message);
                    let mut chain = MessageChainBuilder::group(*group_uin)
                        .text(" ")
                        .text(&res.message)
                        .build();
                    chain.entities.insert(
                        0,
                        Entity::Mention(Mention {
                            uid: gm.uid.clone().into(),
                            name: Some(format!("@{}", gm.group_nickname())),
                            uin: gm.uin,
                        }),
                    );
                    Some(chain)
                }
                Err(e) => {
                    tracing::error!("Failed to upsert member: {:?}", e);
                    Some(
                        MessageChainBuilder::group(*group_uin)
                            .text(&e.message)
                            .build(),
                    )
                }
            }
        }
        "/我没打卡" => {
            debug!("Handling 我没打卡 command for user {}", gm.uin);
            match svc.upsert_member(&gm) {
                Ok(user_id) => {
                    let res = svc.handle_我没打卡(user_id, args);
                    tracing::debug!(
                        "Service handle_我没打卡 ok={} message={}",
                        res.ok,
                        res.message
                    );
                    let mut chain = MessageChainBuilder::group(*group_uin)
                        .text(" ")
                        .text(&res.message)
                        .build();
                    chain.entities.insert(
                        0,
                        Entity::Mention(Mention {
                            uid: gm.uid.clone().into(),
                            name: Some(format!("@{}", gm.group_nickname())),
                            uin: gm.uin,
                        }),
                    );
                    Some(chain)
                }
                Err(e) => {
                    tracing::error!("Failed to upsert member: {:?}", e);
                    Some(
                        MessageChainBuilder::group(*group_uin)
                            .text(&e.message)
                            .build(),
                    )
                }
            }
        }
        "/今日" => {
            let report = svc.build_daily_report();
            Some(MessageChainBuilder::group(*group_uin).text(&report).build())
        }
        "/咕" => {
            let res = svc.handle_咕(*group_uin, &gm, args);
            tracing::debug!("Service handle_咕 ok={} message={}", res.ok, res.message);
            Some(
                MessageChainBuilder::group(*group_uin)
                    .text(" ")
                    .text(&res.message)
                    .build(),
            )
        }
        _ => None,
    }
}

pub async fn run(svc: Service) {
    let config = ClientConfig::default();
    let device = DeviceInfo::load("device.json").unwrap_or_else(|_| {
        tracing::warn!("Failed to load device info, generating a new one...");
        let device = DeviceInfo::default();
        device.save("device.json").unwrap();
        device
    });
    let key_store = KeyStore::load("keystore.json").unwrap_or_else(|_| {
        tracing::warn!("Failed to load keystore, generating a new one...");
        let key_store = KeyStore::default();
        key_store.save("keystore.json").unwrap();
        key_store
    });
    let need_login = key_store.is_expired();
    let mut client = Client::new(config, device, key_store).await.unwrap();

    let op = client.handle().operator().clone();
    let send_op = client.handle().operator().clone();
    let mut group_receiver = op.event_listener.group.clone();
    let mut system_receiver = op.event_listener.system.clone();

    tokio::spawn(async move {
        loop {
            let mut reply = None;
            tokio::select! {
                _ = system_receiver.changed() => {
                    if let Some(ref se) = *system_receiver.borrow() {
                        tracing::info!("[SystemEvent] {:?}", se);
                    }
                }
                _ = group_receiver.changed() => {
                    let guard = group_receiver.borrow();
                    if let Some(ref ge) = *guard {
                        tracing::debug!("[GroupEvent] {:?}", ge);
                        match ge {
                            GroupEvent::GroupMessage(gme) => {
                                if let mania::message::chain::MessageType::Group(_gmeu) = &gme.chain.typ {
                                    reply = handle_group_msg(&svc, gme);
                                }
                            }
                            _ => {},
                        }
                    }
                }
            }
            if let Some(chain) = reply {
                tracing::debug!("Replying with message chain: {:?}", chain);
                if let Err(e) = send_op.send_message(chain).await {
                    tracing::error!("Failed to send message: {:?}", e);
                }
            }
        }
    });

    tokio::spawn(async move {
        client.spawn().await;
    });

    if need_login {
        tracing::warn!("Session is invalid, need to login again!");
        let login_res: Result<(), String> = async {
            let (url, bytes) = op.fetch_qrcode().await.map_err(|e| e.to_string())?;
            let qr_code_name = format!("qrcode.png");
            fs::write(&qr_code_name, &bytes).map_err(|e| e.to_string())?;
            tracing::info!(
                "QR code fetched successfully! url: {}, saved to {}",
                url,
                qr_code_name
            );
            let login_res = op.login_by_qrcode().await.map_err(|e| e.to_string());
            match fs::remove_file(&qr_code_name).map_err(|e| e.to_string()) {
                Ok(_) => tracing::info!("QR code file {} deleted successfully", qr_code_name),
                Err(e) => tracing::error!("Failed to delete QR code file {}: {}", qr_code_name, e),
            }
            login_res
        }
        .await;
        if let Err(e) = login_res {
            tracing::error!("Failed to login: {e:?}");
        }
    } else {
        tracing::info!("Session is still valid, trying to online...");
    }

    let _tx = match op.online().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to set online status: {e:?}");
            return;
        }
    };
    tracing::info!("Bot online");

    op.update_key_store()
        .save("keystore.json")
        .unwrap_or_else(|e| tracing::error!("Failed to save key store: {:?}", e));
}
