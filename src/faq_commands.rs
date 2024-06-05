use sqlx::{Pool, Sqlite};
use std::sync::{Arc, RwLock};
use poise::serenity_prelude as serenity;
use poise::CreateReply;
use log::error;

use crate::{Context, Error, custom_errors::CustomError, util, util::is_mod, SEPARATOR};

#[derive(Debug, Clone)]
pub struct FaqCacheEntry {
    server_id: i64,
    title: String,
}

struct FaqEntry {
    title: String,
    contents: Option<String>,
    image: Option<String>,
    link: Option<String>,
}

pub async fn update_faq_cache(
    cache: Arc<RwLock<Vec<FaqCacheEntry>>>,
    db: Pool<Sqlite>
) -> Result<(), Error> {
    let records = sqlx::query_as!(FaqCacheEntry, r#"SELECT server_id, title FROM faq"#)
        .fetch_all(&db)
        .await?;

    match cache.write() {
        Ok(mut c) => {*c = records},
        Err(e) => {
            return Err(Box::new(CustomError::new(&format!("Error acquiring cache: {e}"))));
        },
    };
    Ok(())
}

pub fn faq() -> poise::Command<crate::Data, Box<dyn std::error::Error + Send + Sync>> {
    poise::Command {
        slash_action: faq_slash().slash_action,
        parameters: faq_slash().parameters,
        ..faq_prefix()
    }
}

/// Frequently Asked Questios
#[allow(clippy::unused_async)]
#[poise::command(slash_command, guild_only)]
pub async fn faq_slash(
    ctx: Context<'_>,
    #[description = "Name of the faq entry"]
    #[autocomplete = "autocomplete_faq"]
    name: String,
) -> Result<(), Error> {
    faq_core(ctx, name).await?;
    Ok(())
}

/// Frequently Asked Questions
#[allow(clippy::unused_async)]
#[poise::command(prefix_command, guild_only, hide_in_help, track_edits, rename = "faq")]
pub async fn faq_prefix(
    ctx: Context<'_>,
    #[description = "Name of the faq entry"]
    #[rest]
    name: Option<String>,
) -> Result<(), Error> {
    if let Some(n) = name {
        faq_core(ctx, n).await?;
    } else {
        list_faqs(ctx).await?;
    }
    Ok(())
}

async fn list_faqs(
    ctx: Context<'_>,
) -> Result<(), Error> {
    let db = &ctx.data().database;
    let server_id = util::get_server_id(ctx)?;
    let db_entries = sqlx::query!(r#"SELECT title FROM faq WHERE server_id = $1"#, server_id)
        .fetch_all(db)
        .await?;
    let mut faq_names = db_entries.iter().map(|f| f.title.clone()).collect::<Vec<String>>();
    faq_names.sort();
    let color = serenity::Colour::GOLD;
    let embed = serenity::CreateEmbed::new()
        .title("List of FAQ tags")
        .description(faq_names.join(", "))
        .color(color);
    let builder = CreateReply::default().embed(embed);
    ctx.send(builder).await?;
    Ok(())
}

async fn faq_core(
    ctx: Context<'_>,
    name: String,
) -> Result<(), Error> {
    let command = name.split(SEPARATOR).next().unwrap_or(&name).trim();
    let name_lc = util::capitalize(&command.to_lowercase());
    let db = &ctx.data().database;
    let server_id = util::get_server_id(ctx)?;

    // Find entry matching given `name`
    let entry_option = sqlx::query_as!(FaqEntry, r#"
        SELECT title, contents, image, link FROM faq WHERE title = $1 AND server_id = $2"#, 
        name_lc, server_id
    )
        .fetch_optional(db)
        .await?;

    // Check if entry found
    let (entry, close_match) = if let Some(e) = entry_option { (e, false) } else {
        // If no entry found, check for near matches
        let closest_match = find_closest_faq(ctx, &name_lc, server_id)?;
        if let Some(match_name) = closest_match {
            (get_faq_entry(db, server_id, &match_name).await?, true)
        } else {
            // If no near matches, return no results message
            let errmsg = format!(
                "Could not find {name_lc} or any similarly tags in FAQ tags. 
                Would you like to search [the wiki](https://wiki.factorio.com/index.php?search={name_lc})?");
            return Err(Box::new(CustomError::new(&errmsg)));
        }
    };

    // If link to other entry found, get other entry
    let entry_final: FaqEntry = match entry.link {
        None => entry,
        Some(entry_link) => {
            get_faq_entry(db, server_id, &entry_link).await?
        }
    };

    // Make and send embed for found entry
    let color = serenity::Colour::GOLD;
    let title = if close_match {
        format!(r#"Could not find "{name_lc}" in FAQ tags. Did you mean "{}"?"#, entry_final.title)
    } else {
        entry_final.title
    };

    let mut embed = serenity::CreateEmbed::new()
        .title(title)
        .color(color);
    if let Some(content) = entry_final.contents {
        embed = embed.description(content);
    }

    if let Some(img) = entry_final.image {
        embed = embed.image(img);
    }

    let builder = CreateReply::default().embed(embed);
    ctx.send(builder).await?;
    Ok(())
}

async fn get_faq_entry(db: &Pool<Sqlite>, server_id: i64, name: &str) -> Result<FaqEntry, Error> {
    Ok(sqlx::query_as!(FaqEntry, r#"
        SELECT title, contents, image, link FROM faq WHERE title = $1 AND server_id = $2"#, 
        name, server_id
    )
        .fetch_optional(db)
        .await?.map_or_else(|| Err(Box::new(CustomError::new(&format!("Could not get FAQ entry {name} from database")))), Ok)?
    )
}

fn find_closest_faq(ctx: Context<'_>, name: &str, server_id: i64) -> Result<Option<String>, Error> {
    let cache = ctx.data().faq_cache.clone();
    let faq_cache = match cache.read() {
        Ok(c) => c,
        Err(e) => {
            return Err(Box::new(CustomError::new(&format!("Error acquiring cache: {e}"))));
        },
    }.clone();
    let server_faqs = faq_cache.iter().filter(|f| f.server_id == server_id).map(|f| f.title.as_str()).collect::<Vec<&str>>();
    let matches = rust_fuzzy_search::fuzzy_search_best_n(name, &server_faqs, 10);
    let best_match = matches.first();
    Ok(best_match
        .filter(|m| m.1 > 0.5)
        .map(|m| m.0.to_owned())
    )
}

#[allow(clippy::unused_async, clippy::cast_possible_wrap)]
async fn autocomplete_faq<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> Vec<String>{
    let Some(server) = ctx.guild_id() else {
        error!("Could not get server ID while autocompleting faq name"); 
        return vec![]
    };
    let server_id = server.get() as i64;
    let cache = ctx.data().faq_cache.clone();
    let faqcache = match cache.read(){
        Ok(c) => c,
        Err(e) => {
            error!{"Error acquiring cache: {e}"}
            return vec![]
        },
    };
    faqcache.iter()
        .filter(|f| f.server_id == server_id && f.title.to_lowercase().starts_with(&partial.to_lowercase()))
        .map(|f| f.title.clone())
        .collect::<Vec<String>>()
}

/// Add, remove or link FAQ entries
#[allow(clippy::unused_async)]
#[poise::command(prefix_command, slash_command, guild_only, check="is_mod", category="Settings", subcommands("new", "remove", "link"), aliases("faq-edit", "faqedit"), subcommand_required)]
pub async fn faq_edit(
    _ctx: Context<'_>
) -> Result<(), Error> {
    Ok(())
}

/// Add an faq entry
#[allow(clippy::unused_async, clippy::cast_possible_wrap)]
#[poise::command(prefix_command, slash_command, guild_only, aliases("edit", "add"))]
pub async fn new(
    ctx: Context<'_>,
    #[description = "Name of the faq"]
    name: String,
    #[description = "Link to an image."]
    image: Option<serenity::Attachment>,
    #[description = "Contents of the FAQ"]
    #[rest]
    content: Option<String>,
) -> Result<(), Error> {
    let name_lc = util::capitalize(&name.to_lowercase());
    let Some(server) = ctx.guild_id() else {
        return Err(Box::new(CustomError::new("Could not get server ID")))
    };
    let server_id = server.get() as i64;
    let db = &ctx.data().database;
    // Check if name already exists
    let pre_existing =  sqlx::query!(r#"SELECT title FROM faq WHERE server_id = $1 AND title = $2"#, server_id, name_lc)
        .fetch_optional(db)
        .await?
        .is_some();

    let mut response: Option<poise::ReplyHandle> = None;

    // If image attached, re-upload image to generate a non-ephemeral link for storage
    let attachment_url = if let Some(att) = image {
        if att.ephemeral {
            let attachment = serenity::CreateAttachment::url(ctx.http(), &att.url).await?;
            let embed = serenity::CreateEmbed::new()
                .title(format!("Adding FAQ entry: {name_lc}"))
                .description("Uploading image to Discord...")
                .colour(serenity::Colour::DARK_GREEN)
                .attachment(att.filename);
            let builder = CreateReply::default().attachment(attachment).embed(embed);
            let r = ctx.send(builder).await?;
            let message = r.message().await?;
            response = Some(r.clone());
            
            let Some(message_embed) = message.embeds.first() else {
                return Err(Box::new(CustomError::new("Could not create FAQ entry: embed not found")))
            };
            let Some(ref embed_image) = message_embed.image else {
                return Err(Box::new(CustomError::new("Could not create FAQ entry: image not found in embed")))
            };
            Some(embed_image.url.clone())
        } else {
            Some(att.url.clone())
        }
    } else {
        None
    };

    let timestamp = ctx.created_at().timestamp();
    let author_id = ctx.author().id.get() as i64;

    // Delete previous entry to prevent duplication
    if pre_existing {
        sqlx::query!(r#"DELETE FROM faq WHERE server_id = $1 AND title = $2"#, server_id, name_lc)
            .execute(db)
            .await?;
    };
    sqlx::query!(r#"INSERT INTO faq (server_id, title, contents, image, edit_time, author)
    VALUES ($1, $2, $3, $4, $5, $6)"#, server_id, name_lc, content, attachment_url, timestamp, author_id)
        .execute(db)
        .await?;

    let title = if pre_existing {format!(r#"Successfully edited "{name_lc}""#)}
        else {format!(r#"Successfully added "{name_lc}" to database"#)};

    let mut embed = serenity::CreateEmbed::new()
        .title(title)
        .colour(serenity::Colour::DARK_GREEN);
    if let Some(c) = content {
        embed = embed.description(c);
    }
    if let Some(url) = attachment_url {
        embed = embed.image(url);
    }
    let builder = CreateReply::default().embed(embed);
    if let Some(r) = response {
        r.edit(ctx, builder).await?;
    } else {
        ctx.send(builder).await?;
    }
    Ok(())
}

/// Remove an faq entry
#[allow(clippy::unused_async, clippy::cast_possible_wrap)]
#[poise::command(prefix_command, slash_command, guild_only, aliases("delete"))]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "FAQ entry to remove"]
    #[autocomplete = "autocomplete_faq"]
    name: String
) -> Result<(), Error> {
    let name_lc = util::capitalize(&name.to_lowercase());
    let Some(server) = ctx.guild_id() else {
        return Err(Box::new(CustomError::new("Could not get server ID")))
    };
    let server_id = server.get() as i64;
    let db = &ctx.data().database;
    match sqlx::query!(r#"DELETE FROM faq WHERE server_id = $1 AND (title = $2 OR link = $2)"#, server_id, name_lc)
        .execute(db)
        .await?
        .rows_affected() {
        0 => {
            ctx.say(format!("FAQ entry {name_lc} does not exist in database")).await?;
        },
        _ => {
            ctx.say(format!("FAQ entry {name_lc} removed from database")).await?;
        },
    };
    Ok(())
}

/// Link two faq titles to the same content
#[allow(clippy::unused_async, clippy::cast_possible_wrap)]
#[poise::command(prefix_command, slash_command, guild_only)]
pub async fn link(
    ctx: Context<'_>,
    #[description = "Name for link"]
    name: String,
    #[autocomplete = "autocomplete_faq"]
    #[description = "Existing FAQ entry to link to"]
    link_to: String,
) -> Result<(), Error> {
    let name_lc = util::capitalize(&name.to_lowercase());
    let link_to_lc = util::capitalize(&link_to.to_lowercase());
    let Some(server) = ctx.guild_id() else {
        return Err(Box::new(CustomError::new("Could not get server ID")))
    };
    let server_id = server.get() as i64;
    let db = &ctx.data().database;

    // Check if name already exists
    if sqlx::query!(r#"SELECT title FROM faq WHERE server_id = $1 AND title = $2"#, server_id, name_lc)
        .fetch_optional(db)
        .await?
        .is_some()
    {
        return Err(Box::new(CustomError::new(&format!("Error: An faq entry with title {name_lc} already exists"))));
    };

    // Find entry to link to
    let linked_entry_opt = sqlx::query_as!(FaqEntry, 
        r#"SELECT title, contents, image, link FROM faq WHERE server_id = $1 AND title = $2"#, server_id, link_to_lc)
        .fetch_optional(db)
        .await?;

    // Check if link target is link. Resolve chain if needed.
    if let Some(linked_entry) = linked_entry_opt {
        let link_no_chain = linked_entry.link.map_or(link_to_lc, |link| link);
        let timestamp = ctx.created_at().timestamp();
        let author_id = ctx.author().id.get() as i64;
        sqlx::query!(r#"INSERT INTO faq (server_id, title, edit_time, author, link)
            VALUES ($1, $2, $3, $4, $5)"#, server_id, name_lc, timestamp, author_id, link_no_chain)
            .execute(db)
            .await?;
        ctx.say(format!("FAQ link {name_lc} added to database, linking to {link_no_chain}")).await?;
    } else {
        return Err(Box::new(CustomError::new(&format!("Error: Could not find FAQ entry {link_to_lc} to link to"))));
    };
    Ok(())
}