use std::iter::once;
use poise::serenity_prelude::{GuildId, RoleId, Permissions};
use sqlx::{Pool, Sqlite};
use crate::{Context, Error};

pub async fn is_mod(ctx: Context<'_>) -> Result<bool, Error> {
    let user_permissions = ctx.author_member().await.unwrap().permissions(ctx.cache()).unwrap();
    if user_permissions.contains(Permissions::ADMINISTRATOR) {
        return Ok(true);
    };
    let db = &ctx.data().database;
    let server_id = ctx.guild_id().unwrap().get() as i64;
    let modrole = match sqlx::query!(r#"SELECT modrole FROM servers WHERE server_id = ?1"#, server_id)
        .fetch_one(db)
        .await {
            Ok(role) => {match role.modrole {
                Some(role) => role,
                None => {
                    return Ok(false)
                },
            }},
            Err(_) => {
                return Ok(false)
            },
        };
    let has_role = ctx.author().has_role(ctx.http(), ctx.guild_id().unwrap(), RoleId::from(modrole as u64)).await?;
    Ok(has_role)
}

pub async fn escape_formatting(unformatted_string: String) -> String {
    // This is supposedly cheaper than using the String::replace function.
    unformatted_string
        .chars()
        .into_iter()
        .flat_map(|c| match c {
            '_' | '*' | '~' => Some('\\'),
            _ => None
        }
            .into_iter()
            .chain(once(c))
        )
        .flat_map(|c| once(c).chain( match c {
            '@' => Some('\u{200b}'),
            _ => None
        }))
        .collect::<String>()
}

pub async fn get_subscribed_mods(db: &Pool<Sqlite>, server_id: i64) -> Result<Vec<String>, Error> {
    let subscribed_mods = sqlx::query!(r#"SELECT mod_name FROM subscribed_mods WHERE server_id = ?1"#, server_id)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|m| m.mod_name.unwrap())
        .collect::<Vec<String>>();
    return Ok(subscribed_mods);
}
pub async fn get_subscribed_authors(db: &Pool<Sqlite>, server_id: i64) -> Result<Vec<String>, Error> {
    let subscribed_authors = sqlx::query!(r#"SELECT author_name FROM subscribed_authors WHERE server_id = ?1"#, server_id)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|m| m.author_name.unwrap())
        .collect::<Vec<String>>();
    return Ok(subscribed_authors)
}

/// Show stored information about this server
#[poise::command(prefix_command, slash_command, guild_only, ephemeral, category="Settings")]
pub async fn get_server_info(
    ctx: Context<'_>
) -> Result<(), Error> {
    let server_id = ctx.guild_id().unwrap().get() as i64;
    let db = &ctx.data().database;
    let serverdata = sqlx::query!(r#"SELECT * FROM servers WHERE server_id = ?1"#, server_id)
        .fetch_optional(db)
        .await?;
    match serverdata {
        Some(data) => {
            let updates_channel = match data.updates_channel {
                Some(ch) => format!("<#{ch}>"),
                None => "Not set".to_owned(),
            };
            let modrole = match data.modrole {
                Some(role) => format!("<@&{role}>"),
                None => "Not set".to_owned(),
            };
            let show_changelog = match data.show_changelog {
                Some(b) => b.to_string(),
                None => "Not set (default to true)".to_owned(),
            };
            let response = format!("**Stored information for this server:**\nServer ID: {:?}\nUpdates channel: {}\nmodrole: {}\nShow changelogs: {}",
                data.server_id.unwrap_or(0), updates_channel, modrole, show_changelog);
            ctx.say(response).await?;
        },
        None => {
            ctx.say("No data stored about this server").await?;
        },
    }
    Ok(())
}

/// Show this help menu
#[poise::command(prefix_command, track_edits, slash_command)]
pub async fn help(
    ctx: Context<'_>,
    #[description = "Specific command to show help about"]
    #[autocomplete = "poise::builtins::autocomplete_command"]
    command: Option<String>,
) -> Result<(), Error> {
    poise::builtins::help(
        ctx,
        command.as_deref(),
        poise::builtins::HelpConfiguration {
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Remove all stored data for this server, resetting all settings.
#[poise::command(prefix_command, slash_command, guild_only, category="Settings", check="is_mod")]
pub async fn reset_server_settings(
    ctx: Context<'_>
) -> Result<(), Error> {
    let server_id = ctx.guild_id().unwrap().get() as i64;
    let db = &ctx.data().database;
    sqlx::query!(r#"DELETE FROM servers WHERE server_id = ?1"#, server_id)
        .execute(db)
        .await?;
    ctx.say("Server data reset").await?;
    Ok(())
}

// /// Manually add entries to the database. Owner only.
// #[poise::command(prefix_command, slash_command, guild_only, owners_only, category="Management")]
// pub async fn migrate_serverdb_entry(
//     ctx: Context<'_>,
//     server_id: String,
//     updates_channel: Option<String>,
//     mod_role: Option<String>,
//     subscribed_mods: Option<String>,
// ) -> Result<(), Error> {
//     let db = &ctx.data().database;
//     let id = server_id.parse::<i64>().unwrap();
//     let ch = match updates_channel {
//         Some(c) => Some(c.parse::<i64>().unwrap()),
//         None => None,
//     };
//     let role = match mod_role {
//         Some(r) => Some(r.parse::<i64>().unwrap()),
//         None => None,
//     };

//     sqlx::query!(r#"INSERT INTO servers (server_id, updates_channel, modrole) VALUES (?1, ?2, ?3)"#, id, ch, role)
//         .execute(db)
//         .await?;
//     if subscribed_mods.is_some() {
//         let unwrapped_mods = subscribed_mods.unwrap();
//         let mods = unwrapped_mods.split(", ").collect::<Vec<&str>>();
//         for modname in mods {
//             sqlx::query!(r#"INSERT INTO subscribed_mods (server_id, mod_name) VALUES (?1, ?2)"#, server_id, modname)
//             .execute(db)
//             .await?;
//         };
//     };
//     ctx.say(format!("entry for server {server_id} added to database")).await?;
//     Ok(())
// }

pub async fn on_guild_leave(id: GuildId, db: Pool<Sqlite>) -> Result<(), Error> {
    let server_id = id.get() as i64;
    sqlx::query!(r#"DELETE FROM servers WHERE server_id = ?1"#, server_id)
        .execute(&db)
        .await?;
    sqlx::query!(r#"DELETE FROM subscribed_mods WHERE server_id = ?1"#, server_id)
        .execute(&db)
        .await?;
    sqlx::query!(r#"DELETE FROM subscribed_authors WHERE server_id = ?1"#, server_id)
        .execute(&db)
        .await?;
    sqlx::query!(r#"DELETE FROM faq WHERE server_id = ?1"#, server_id)
        .execute(&db)
        .await?;
    println!("Left guild {server_id}");
    Ok(())
}