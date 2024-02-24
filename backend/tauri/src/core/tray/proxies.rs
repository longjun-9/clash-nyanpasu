use crate::core::{
    clash::{
        api::ProxyItem,
        proxies::{ProxiesGuard, ProxiesGuardExt},
    },
    handle::Handle,
    tray::Tray,
};
use log::{debug, error, warn};
use std::collections::BTreeMap;
use tauri::SystemTrayMenu;

async fn loop_task() {
    loop {
        match ProxiesGuard::global().update().await {
            Ok(_) => {
                debug!(target: "tray", "update proxies success");
            }
            Err(e) => {
                warn!(target: "tray", "update proxies failed: {:?}", e);
            }
        }
        {
            let guard = ProxiesGuard::global().read();
            if guard.updated_at() == 0 {
                error!(target: "tray", "proxies not updated yet!!!!");
                // TODO: add a error dialog or notification, and panic?
            } else {
                let proxies = guard.inner();
                let str = simd_json::to_string_pretty(proxies).unwrap();
                debug!(target: "tray", "proxies info: {:?}", str);
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await; // TODO: add a config to control the interval
    }
}

type GroupName = String;
type FromProxy = String;
type ToProxy = String;
#[derive(PartialEq)]
enum TrayUpdateType {
    None,
    Full,
    Part(Vec<(GroupName, FromProxy, ToProxy)>),
}

fn diff_proxies(
    old_proxies: &BTreeMap<String, ProxyItem>,
    new_proxies: &BTreeMap<String, ProxyItem>,
) -> TrayUpdateType {
    let mut update_mode = TrayUpdateType::None;
    let group_matching = new_proxies
        .clone()
        .into_keys()
        .collect::<Vec<String>>()
        .iter()
        .zip(&old_proxies.clone().into_keys().collect::<Vec<String>>())
        .filter(|&(new, old)| new == old)
        .count();
    if group_matching == old_proxies.len() && group_matching == new_proxies.len() {
        let mut action_list = Vec::<(GroupName, FromProxy, ToProxy)>::new();
        // Iterate through two btreemap
        for (group_key, new_proxy) in new_proxies {
            match old_proxies.get(group_key) {
                Some(old_proxy) => {
                    if old_proxy.now.is_some()
                        && new_proxy.now.is_some()
                        && old_proxy.now != new_proxy.now
                    {
                        action_list.insert(
                            0,
                            (
                                group_key.to_owned(),
                                old_proxy.now.as_ref().unwrap().to_owned(),
                                new_proxy.now.as_ref().unwrap().to_owned(),
                            ),
                        );
                    }
                }
                None => {
                    update_mode = TrayUpdateType::Full;
                    break;
                }
            }
        }
        if update_mode != TrayUpdateType::Full && !action_list.is_empty() {
            update_mode = TrayUpdateType::Part(action_list);
        }
    } else {
        update_mode = TrayUpdateType::Full
    }
    update_mode
}

pub async fn proxies_updated_receiver() {
    let mut rx = ProxiesGuard::global().read().get_receiver();
    let mut old_proxies: BTreeMap<String, ProxyItem> = ProxiesGuard::global()
        .read()
        .inner()
        .to_owned()
        .records
        .into_iter()
        .collect();
    loop {
        match rx.recv().await {
            Ok(_) => {
                debug!(target: "tray::proxies", "proxies updated");
                if Handle::global().app_handle.lock().is_none() {
                    warn!(target: "tray::proxies", "app handle not found");
                    continue;
                }
                Handle::mutate_proxies();
                // Do diff check
                let new_proxies: BTreeMap<String, ProxyItem> = ProxiesGuard::global()
                    .read()
                    .inner()
                    .to_owned()
                    .records
                    .into_iter()
                    .collect();

                match diff_proxies(&old_proxies, &new_proxies) {
                    TrayUpdateType::None => {}
                    TrayUpdateType::Full => {
                        old_proxies = new_proxies;
                        // println!("{}", simd_json::to_string_pretty(&proxies).unwrap());
                        match Handle::update_systray() {
                            Ok(_) => {
                                debug!(target: "tray::proxies", "update systray success");
                            }
                            Err(e) => {
                                warn!(target: "tray::proxies", "update systray failed: {:?}", e);
                            }
                        }
                    }
                    TrayUpdateType::Part(action_list) => {
                        old_proxies = new_proxies;
                        for action in action_list {
                            match Tray::update_selected_proxy(
                                format!("{}_{}", action.0, action.1),
                                format!("{}_{}", action.0, action.2),
                            ) {
                                Ok(_) => {
                                    debug!(target: "tray::proxies", "update systray part success");
                                }
                                Err(e) => {
                                    warn!(target: "tray::proxies", "update systray part failed: {:?}", e);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(target: "tray::proxies", "proxies updated receiver failed: {:?}", e);
            }
        }
    }
}

pub fn setup_proxies() {
    tauri::async_runtime::spawn(loop_task());
    tauri::async_runtime::spawn(proxies_updated_receiver());
}

mod platform_impl {
    use crate::core::clash::proxies::{ProxiesGuard, ProxyGroupItem};
    use tauri::{CustomMenuItem, SystemTrayMenu, SystemTraySubmenu};

    pub fn generate_group_selector(group: &ProxyGroupItem) -> SystemTraySubmenu {
        let mut group_menu = SystemTrayMenu::new();
        for item in group.all.iter() {
            let mut sub_item = CustomMenuItem::new(
                format!("select_proxy_{}_{}", group.name, item.name.clone()),
                item.name.clone(),
            );
            if let Some(now) = group.now.clone() {
                if now == item.name {
                    sub_item = sub_item.selected();
                }
            }
            group_menu = group_menu.add_item(sub_item);
        }
        SystemTraySubmenu::new(group.name.clone(), group_menu)
    }

    pub fn setup_tray(menu: &mut SystemTrayMenu) -> SystemTrayMenu {
        let proxies = ProxiesGuard::global().read().inner().to_owned();
        let mode = crate::utils::config::get_current_clash_mode();
        let mut menu = menu.to_owned();
        match mode.as_str() {
            "rule" | "script" | "global" => {
                if mode == "global" || proxies.groups.is_empty() {
                    let group_selector = generate_group_selector(&proxies.global);
                    menu = menu.add_submenu(group_selector);
                }
                for group in proxies.groups.iter() {
                    let group_selector = generate_group_selector(group);
                    menu = menu.add_submenu(group_selector);
                }
                menu
            }
            _ => {
                menu.add_item(CustomMenuItem::new("no_proxy", "NO PROXY COULD SELECTED").disabled())
                // DIRECT
            }
        }
    }
}

pub trait SystemTrayMenuProxiesExt {
    fn setup_proxies(&mut self) -> Self;
}

impl SystemTrayMenuProxiesExt for SystemTrayMenu {
    fn setup_proxies(&mut self) -> Self {
        platform_impl::setup_tray(self)
    }
}

pub fn on_system_tray_event(event: &str) {
    if !event.starts_with("select_proxy_") {
        return; // bypass non-select event
    }
    let parts: Vec<&str> = event.split('_').collect();
    if parts.len() != 4 {
        return; // bypass invalid event
    }
    let group = parts[2].to_owned();
    let name = parts[3].to_owned();
    tauri::async_runtime::spawn(async move {
        match ProxiesGuard::global().select_proxy(&group, &name).await {
            Ok(_) => {
                debug!(target: "tray", "select proxy success: {} {}", group, name);
            }
            Err(e) => {
                warn!(target: "tray", "select proxy failed, {} {}, cause: {:?}", group, name, e);
                // TODO: add a error dialog or notification
            }
        }
    });
}
