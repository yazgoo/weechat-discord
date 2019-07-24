use crate::{
    ffi::{get_option, update_bar_item, Buffer},
    printing, utils,
};
use lazy_static::lazy_static;
use serenity::{cache::CacheRwLock, model::prelude::*};
use std::collections::{HashMap, VecDeque};

lazy_static! {
    static ref OFFLINE_GROUP_NAME: String = format!("{}|Offline", ::std::i64::MAX);
    static ref ONLINE_GROUP_NAME: String = format!("{}|Online", ::std::i64::MAX - 1);
    static ref BOT_GROUP_NAME: String = format!("{}|{}", ::std::i64::MAX, "Bot");
}

pub fn create_buffers(ready_data: &Ready) {
    let ctx = match crate::discord::get_ctx() {
        Some(ctx) => ctx,
        _ => return,
    };
    let current_user = ctx.cache.read().user.clone();

    let guilds = match current_user.guilds(ctx) {
        Ok(guilds) => guilds,
        Err(e) => {
            on_main! {{
                crate::plugin_print(&format!("Error getting user guilds: {:?}", e));
            }};
            vec![]
        }
    };
    let mut map: HashMap<_, _> = guilds.iter().map(|g| (g.id, g)).collect();

    let mut sorted_guilds = VecDeque::new();

    // Add the guilds ordered from the client
    for guild_id in &ready_data.user_settings.guild_positions {
        if let Some(guild) = map.remove(&guild_id) {
            sorted_guilds.push_back(guild);
        }
    }

    // Prepend any remaning guilds
    for guild in map.values() {
        sorted_guilds.push_front(guild);
    }

    for guild in &sorted_guilds {
        let guild_settings = ready_data.user_guild_settings.get(&guild.id.into());
        let guild_muted;
        let mut channel_muted = HashMap::new();
        if let Some(guild_settings) = guild_settings {
            guild_muted = guild_settings.muted;
            for (channel_id, channel_override) in guild_settings.channel_overrides.iter() {
                channel_muted.insert(channel_id, channel_override.muted);
            }
        } else {
            guild_muted = false;
        }
        create_guild_buffer(guild.id, &guild.name);

        // TODO: Colors?
        let nick = if let Ok(current_member) = guild.id.member(ctx, current_user.id) {
            format!("@{}", current_member.display_name())
        } else {
            format!("@{}", current_user.name)
        };
        let channels = guild.id.channels(ctx).expect("Unable to fetch channels");
        let mut channels = channels.values().collect::<Vec<_>>();
        channels.sort_by_key(|g| g.position);
        for channel in channels {
            let is_muted =
                guild_muted || channel_muted.get(&channel.id).cloned().unwrap_or_default();
            create_buffer_from_channel(&ctx.cache, &channel, &nick, is_muted);
        }
    }
}

// TODO: Merge these functions
// Flesh this out
pub fn create_autojoin_buffers(_ready: &Ready) {
    let ctx = match crate::discord::get_ctx() {
        Some(ctx) => ctx,
        _ => return,
    };

    let current_user = ctx.cache.read().user.clone();

    let autojoin_items = match get_option("autojoin_channels") {
        Some(items) => items,
        None => return,
    };

    let autojoin_items = autojoin_items
        .split(',')
        .filter(|i| !i.is_empty())
        .filter_map(utils::parse_id);

    let mut channels = Vec::new();
    // flatten guilds into channels
    for item in autojoin_items {
        match item {
            utils::GuildOrChannel::Guild(guild_id) => {
                let guild_channels = guild_id.channels(ctx).expect("Unable to fetch channels");
                let mut guild_channels = guild_channels.values().collect::<Vec<_>>();
                guild_channels.sort_by_key(|g| g.position);
                channels.extend(guild_channels.iter().map(|ch| (Some(guild_id), ch.id)));
            }
            utils::GuildOrChannel::Channel(guild, channel) => channels.push((guild, channel)),
        }
    }

    // TODO: Flatten and iterate by guild, then channel
    for (guild_id, channel_id) in &channels {
        if let Some(guild_id) = guild_id {
            let guild = match guild_id.to_guild_cached(&ctx.cache) {
                Some(guild) => guild,
                None => continue,
            };
            let guild = guild.read();

            let channel = match guild.channels.get(channel_id) {
                Some(channel) => channel,
                None => continue,
            };
            let channel = channel.read();

            // TODO: Colors?
            let nick = if let Ok(current_member) = guild.id.member(ctx, current_user.id) {
                format!("@{}", current_member.display_name())
            } else {
                format!("@{}", current_user.name)
            };

            create_guild_buffer(guild.id, &guild.name);
            // TODO: Muting
            create_buffer_from_channel(&ctx.cache, &channel, &nick, false)
        }
    }
}

pub fn create_guild_buffer_lockable(id: GuildId, name: &str, lock: bool) {
    let guild_name_id = utils::buffer_id_for_guild(id);
    on_main!(lock, {
        let buffer = if let Some(buffer) = Buffer::search(&guild_name_id) {
            buffer
        } else {
            Buffer::new(&guild_name_id, |_, _| {}).unwrap()
        };
        buffer.set("short_name", name);
        buffer.set("localval_set_guildid", &id.0.to_string());
        buffer.set("localvar_set_type", "server");
    });
}

pub fn create_guild_buffer(id: GuildId, name: &str) {
    create_guild_buffer_lockable(id, name, true);
}

pub fn create_buffer_from_channel_lockable(
    cache: &CacheRwLock,
    channel: &GuildChannel,
    nick: &str,
    muted: bool,
    lock: bool,
) {
    let current_user = cache.read().user.clone();
    if let Ok(perms) = channel.permissions_for(cache, current_user.id) {
        if !perms.read_message_history() {
            return;
        }
    }

    let channel_type = match channel.kind {
        ChannelType::Category | ChannelType::Voice => return,
        ChannelType::Private => "private",
        ChannelType::Group | ChannelType::Text | ChannelType::News => "channel",
        _ => panic!("Unknown chanel type"),
    };

    let name_id = utils::buffer_id_for_channel(Some(channel.guild_id), channel.id);

    on_main!(lock, {
        let buffer = if let Some(buffer) = Buffer::search(&name_id) {
            buffer
        } else {
            Buffer::new(&name_id, crate::hook::buffer_input).unwrap()
        };
        buffer.set("short_name", &channel.name);
        buffer.set("localvar_set_channelid", &channel.id.0.to_string());
        buffer.set("localvar_set_guildid", &channel.guild_id.0.to_string());
        buffer.set("localvar_set_type", channel_type);
        buffer.set("localvar_set_nick", &nick);
        let mut title = if let Some(ref topic) = channel.topic {
            if !topic.is_empty() {
                format!("{} | {}", channel.name, topic)
            } else {
                channel.name.clone()
            }
        } else {
            channel.name.clone()
        };

        if muted {
            title += " (muted)";
        }
        buffer.set("title", &title);
        buffer.set("localvar_set_muted", &(muted as u8).to_string());
    });
}

pub fn create_buffer_from_channel(
    cache: &CacheRwLock,
    channel: &GuildChannel,
    nick: &str,
    muted: bool,
) {
    create_buffer_from_channel_lockable(cache, channel, nick, muted, true)
}

// TODO: Reduce code duplication
/// Must be called on main
pub fn create_buffer_from_dm(channel: Channel, nick: &str, switch_to: bool) {
    let channel = match channel.private() {
        Some(chan) => chan,
        None => return,
    };
    let channel = channel.read();

    let name_id = utils::buffer_id_for_channel(None, channel.id);
    let buffer = if let Some(buffer) = Buffer::search(&name_id) {
        buffer
    } else {
        Buffer::new(&name_id, crate::hook::buffer_input).unwrap()
    };

    buffer.set("short_name", &channel.name());
    buffer.set("localvar_set_channelid", &channel.id.0.to_string());
    buffer.set("localvar_set_nick", &nick);
    if switch_to {
        buffer.set("display", "1");
    }
    let title = format!("DM with {}", channel.recipient.read().name);
    buffer.set("title", &title);
}

/// Must be called on main
pub fn create_buffer_from_group(channel: Channel, nick: &str) {
    let channel = match channel.group() {
        Some(chan) => chan,
        None => return,
    };
    let channel = channel.read();

    let title = format!(
        "DM with {}",
        channel
            .recipients
            .values()
            .map(|u| u.read().name.to_owned())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let name_id = utils::buffer_id_for_channel(None, channel.channel_id);

    let buffer = if let Some(buffer) = Buffer::search(&name_id) {
        buffer
    } else {
        Buffer::new(&name_id, crate::hook::buffer_input).unwrap()
    };

    buffer.set("short_name", &channel.name());
    buffer.set("localvar_set_channelid", &channel.channel_id.0.to_string());
    buffer.set("localvar_set_nick", &nick);
    buffer.set("title", &title);
}

// TODO: Make this nicer somehow
// TODO: Refactor this to use `?`
pub fn load_nicks(buffer: &Buffer) {
    let (guild_id, channel_id, use_presence) = on_main! {{
        if buffer.get("localvar_loaded_nicks").is_some() {
            return;
        }

        let guild_id = match buffer.get("localvar_guildid") {
            Some(guild_id) => guild_id,
            None => return,
        };

        let channel_id = match buffer.get("localvar_channelid") {
            Some(channel_id) => channel_id,
            None => return,
        };

        let guild_id = match guild_id.parse::<u64>() {
            Ok(v) => GuildId(v),
            Err(_) => return,
        };

        let channel_id = match channel_id.parse::<u64>() {
            Ok(v) => ChannelId(v),
            Err(_) => return,
        };

        buffer.set("localvar_set_loaded_nicks", "true");
        buffer.set("nicklist", "1");

        let use_presence = get_option("use_presence").map(|o| o == "true").unwrap_or(false);

        (guild_id, channel_id, use_presence)
    }};
    let ctx = match crate::discord::get_ctx() {
        Some(ctx) => ctx,
        _ => return,
    };

    let guild = guild_id.to_guild_cached(ctx).expect("No guild cache item");

    let current_user = ctx.cache.read().user.id;

    // Typeck not smart enough
    let none_user: Option<UserId> = None;
    // TODO: What to do with more than 1000 members?
    let members = guild.read().members(ctx, Some(1000), none_user).unwrap();
    on_main! {{
        for member in members {
            let user = member.user.read();
            // the current user does not seem to usually have a presence, assume they are online
            let online = if !use_presence {
                // Dont do the lookup
                false
            } else if user.id == current_user {
                true
            } else {
                let cache = ctx.cache.read();
                let presence = cache.presences.get(&member.user_id());
                presence
                    .map(|p| utils::status_is_online(p.status))
                    .unwrap_or(false)
            };

            let member_perms = guild.read().permissions_in(channel_id, user.id);
            // A pretty accurate method of checking if a user is "in" a channel
            if !member_perms.read_message_history() || !member_perms.read_messages() {
                continue;
            }

            let role_name;
            let role_color;

            // TODO: Change offline/online color somehow?
            if user.bot {
                role_name = BOT_GROUP_NAME.clone();
                role_color = "gray".to_string();
            } else if !online && use_presence {
                role_name = OFFLINE_GROUP_NAME.clone();
                role_color = "grey".to_string();
            } else if let Some((highest_hoisted, highest)) =
                utils::find_highest_roles(&ctx.cache, &member)
            {
                role_name = format!(
                    "{}|{}",
                    ::std::i64::MAX - highest_hoisted.position,
                    highest_hoisted.name
                );
                role_color = crate::utils::rgb_to_ansi(highest.colour).to_string();
            } else {
                // Can't find a role, add user to generic bucket
                if use_presence {
                    if online {
                        role_name = ONLINE_GROUP_NAME.clone();
                    } else {
                        role_name = OFFLINE_GROUP_NAME.clone();
                    }
                    role_color = "grey".to_string();
                } else {
                    buffer.add_nick(member.display_name().as_ref());
                    continue;
                }
            }
            if !buffer.group_exists(&role_name) {
                buffer.add_nicklist_group_with_color(&role_name, &role_color);
            }
            buffer.add_nick_to_group(member.display_name().as_ref(), &role_name);
        }
    }};
}

pub fn load_history(buffer: &Buffer) {
    let channel = on_main! {{
        if buffer.get("localvar_loaded_history").is_some() {
            return;
        }
        let channel = match buffer.get("localvar_channelid") {
            Some(channel) => channel,
            None => {
                return;
            }
        };
        let channel = match channel.parse::<u64>() {
            Ok(v) => ChannelId(v),
            Err(_) => return,
        };
        buffer.clear();
        buffer.set("localvar_set_loaded_history", "true");
        channel
    }};

    let ctx = match crate::discord::get_ctx() {
        Some(ctx) => ctx,
        _ => return,
    };

    if let Ok(msgs) = channel.messages(ctx, |retriever| retriever.limit(25)) {
        on_main! {{
            for msg in msgs.into_iter().rev() {
                printing::print_msg(&buffer, &msg, false);
            }
        }};
    }
}

pub fn update_nick() {
    let ctx = match crate::discord::get_ctx() {
        Some(ctx) => ctx,
        _ => return,
    };
    let current_user = ctx.cache.read().user.clone();

    for guild in current_user.guilds(ctx).expect("Unable to fetch guilds") {
        // TODO: Colors?
        let nick = if let Ok(current_member) = guild.id.member(ctx, current_user.id) {
            format!("@{}", current_member.display_name())
        } else {
            format!("@{}", current_user.name)
        };

        let channels = guild.id.channels(ctx).expect("Unable to fetch channels");
        for channel_id in channels.keys() {
            let string_channel = utils::buffer_id_for_channel(Some(guild.id), *channel_id);
            if let Some(buffer) = Buffer::search(&string_channel) {
                buffer.set("localvar_set_nick", &nick);
                update_bar_item("input_prompt");
            }
        }
    }
}
