#[macro_use] extern crate rocket;

use rocket::form::Form;
use rocket::fs::{relative, FileServer, TempFile};
use rocket::http::ContentType;
use rocket::response::{content::RawHtml, Redirect};
use rocket::serde::{Serialize, Deserialize};
use rusqlite::{params, Connection};
use rand::{distributions::Alphanumeric, Rng};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use rocket::fairing::AdHoc;
use rocket::Config;

#[derive(Debug, Serialize, Deserialize, FromForm)]
struct Post {
    id: Option<i32>,
    content: String,
    parent_id: Option<i32>,
    reply_id: Option<i32>,
    display_id: Option<String>,
    timestamp: Option<u64>,
    image_url: Option<String>,
}

#[derive(FromForm)]
struct PostForm<'r> {
    content: &'r str,
    image: Option<TempFile<'r>>,
}

fn generate_display_id() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(5)
        .map(char::from)
        .collect()
}

fn current_timestamp() -> u64 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
    since_the_epoch.as_secs()
}

fn get_extension(content_type: &ContentType) -> Option<&str> {
    if content_type == &ContentType::JPEG {
        Some("jpg")
    } else if content_type == &ContentType::PNG {
        Some("png")
    } else if content_type == &ContentType::GIF {
        Some("gif")
    } else if content_type == &ContentType::WEBP {
        Some("webp")
    } else {
        None
    }
}

#[post("/submit", data = "<post_form>")]
async fn submit(mut post_form: Form<PostForm<'_>>) -> Result<Redirect, String> {
    let content = post_form.content.to_string();
    let display_id = generate_display_id();
    let timestamp = current_timestamp();
    let mut image_url = None;

    if let Some(image) = &mut post_form.image {
        if let Some(ext) = image.content_type().and_then(get_extension) {
            let filename = format!("{}.{}", display_id, ext);
            let filepath = Path::new("static/uploads").join(&filename);
            match image.persist_to(filepath).await {
                Ok(_) => {
                    image_url = Some(format!("/static/uploads/{}", filename));
                }
                Err(e) => {
                    let error_message = format!("Failed to save image: {}", e);
                    eprintln!("{}", error_message);
                    return Err(error_message);
                }
            }
        }
    }

    let conn = match Connection::open("posts.db") {
        Ok(conn) => conn,
        Err(e) => {
            let error_message = format!("Failed to open database connection: {}", e);
            eprintln!("{}", error_message);
            return Err(error_message);
        }
    };

    if let Err(e) = conn.execute(
        "INSERT INTO posts (content, parent_id, reply_id, display_id, timestamp, image_url) VALUES (?1, NULL, NULL, ?2, ?3, ?4)",
        params![content, display_id, timestamp, image_url],
    ) {
        let error_message = format!("Failed to insert post into database: {}", e);
        eprintln!("{}", error_message);
        return Err(error_message);
    }

    Ok(Redirect::to("/"))
}

#[post("/submit_reply/<parent_id>", data = "<post_form>")]
async fn submit_reply(parent_id: i32, post_form: Form<PostForm<'_>>) -> Result<Redirect, String> {
    let content = post_form.content.to_string();
    let timestamp = current_timestamp();

    let conn = match Connection::open("posts.db") {
        Ok(conn) => conn,
        Err(e) => {
            let error_message = format!("Failed to open database connection: {}", e);
            eprintln!("{}", error_message);
            return Err(error_message);
        }
    };

    let reply_id: i32 = match conn.query_row(
        "SELECT COALESCE(MAX(reply_id), 0) + 1 FROM posts WHERE parent_id = ?1",
        params![parent_id],
        |row| row.get(0)
    ) {
        Ok(id) => id,
        Err(e) => {
            let error_message = format!("Failed to get next reply_id: {}", e);
            eprintln!("{}", error_message);
            return Err(error_message);
        }
    };

    if let Err(e) = conn.execute(
        "INSERT INTO posts (content, parent_id, reply_id, display_id, timestamp) VALUES (?1, ?2, ?3, NULL, ?4)",
        params![content, parent_id, reply_id, timestamp],
    ) {
        let error_message = format!("Failed to insert reply into database: {}", e);
        eprintln!("{}", error_message);
        return Err(error_message);
    }

    // Update the timestamp of the original post to bring it to the top
    if let Err(e) = conn.execute(
        "UPDATE posts SET timestamp = ?1 WHERE id = ?2",
        params![timestamp, parent_id],
    ) {
        let error_message = format!("Failed to update post timestamp: {}", e);
        eprintln!("{}", error_message);
        return Err(error_message);
    }

    Ok(Redirect::to(format!("/reply/{}", parent_id)))
}

#[get("/?<page>")]
fn index(page: Option<usize>) -> RawHtml<String> {
    let page = page.unwrap_or(1);
    let posts_per_page = 10;
    let offset = (page - 1) * posts_per_page;

    let conn = Connection::open("posts.db").unwrap();
    let mut stmt = conn.prepare("SELECT id, content, display_id, image_url FROM posts WHERE parent_id IS NULL ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2").unwrap();
    let post_iter = stmt.query_map(params![posts_per_page as i64, offset as i64], |row| {
        Ok(Post {
            id: row.get(0)?,
            content: row.get(1)?,
            parent_id: None,
            reply_id: None,
            display_id: row.get(2)?,
            timestamp: None,
            image_url: row.get(3)?,
        })
    }).unwrap();

    let mut posts = String::new();
    for post in post_iter {
        let post = post.unwrap();
        let reply_count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM posts WHERE parent_id = ?1",
            params![post.id],
            |row| row.get(0)
        ).unwrap();
        posts.push_str(&format!(
            "<div class='post'>
                <div class='post-header'>
                    <span class='post-id'>{}</span>
                    <a href='/reply/{}' class='reply-button'>Reply ({})</a>
                </div>
                {}
                <div class='post-content'>
                    {}
                </div>
            </div>",
            post.display_id.as_ref().unwrap(), post.id.unwrap(), reply_count,
            if let Some(image_url) = post.image_url {
                format!("<img src='{}' alt='Image' class='responsive-img'/>", image_url)
            } else {
                String::new()
            },
            post.content.replace("\n", "<br/>")
        ));
    }

    let mut pagination = String::new();
    if page > 1 {
        pagination.push_str(&format!(r#"<a href="/?page={}" class="button">Previous</a>"#, page - 1));
    }
    pagination.push_str(&format!(r#"<a href="/?page={}" class="button">Next</a>"#, page + 1));

    RawHtml(format!(
        r#"
        <html>
            <head>
                <link rel="stylesheet" type="text/css" href="/static/styles.css">
            </head>
            <body>
                <div class="container">
                    <form action="/submit" method="post" enctype="multipart/form-data">
                        <textarea name="content" required></textarea><br/>
                        <input type="file" name="image" accept="image/jpeg, image/png, image/gif, image/webp"><br/>
                        <input type="submit" value="Post" class="button">
                    </form>
                    <div class="posts">{}</div>
                    <div class="pagination">{}</div>
                </div>
            </body>
        </html>
        "#,
        posts,
        pagination
    ))
}

#[get("/reply/<post_id>")]
fn reply(post_id: i32) -> RawHtml<String> {
    let conn = Connection::open("posts.db").unwrap();
    
    let mut stmt = conn.prepare("SELECT id, content, display_id, image_url FROM posts WHERE id = ?1").unwrap();
    let post = stmt.query_row(params![post_id], |row| {
        Ok(Post {
            id: row.get(0)?,
            content: row.get(1)?,
            parent_id: None,
            reply_id: None,
            display_id: row.get(2)?,
            timestamp: None,
            image_url: row.get(3)?,
        })
    }).unwrap();

    let mut stmt = conn.prepare("SELECT id, content, reply_id FROM posts WHERE parent_id = ?1 ORDER BY reply_id DESC").unwrap();
    let reply_iter = stmt.query_map(params![post_id], |row| {
        Ok(Post {
            id: row.get(0)?,
            content: row.get(1)?,
            parent_id: Some(post_id),
            reply_id: row.get(2)?,
            display_id: None,
            timestamp: None,
            image_url: None,
        })
    }).unwrap();

    let mut replies = String::new();
    for reply in reply_iter {
        let reply = reply.unwrap();
        replies.push_str(&format!(
            "<div class='post'>
                <div class='post-header'>
                    <span class='post-id'>Reply {}</span>
                </div>
                <div class='post-content'>
                    {}
                </div>
            </div>",
            reply.reply_id.unwrap(),
            reply.content.replace("\n", "<br/>")
        ));
    }

    RawHtml(format!(
        r#"
        <html>
            <head>
                <link rel="stylesheet" type="text/css" href="/static/styles.css">
            </head>
            <body>
                <div class="container">
                    <a href="/" class="home-button">Home</a>
                    <form action="/submit_reply/{}" method="post">
                        <textarea name="content" required></textarea><br/>
                        <input type="submit" value="Reply" class="button">
                    </form>
                    <div class="post">
                        <div class='post-header'>
                            <span class='post-id'>{}</span>
                        </div>
                        {}
                        <div class='post-content'>
                            {}
                        </div>
                    </div>
                    <div class="replies">{}</div>
                </div>
            </body>
        </html>
        "#,
        post_id,
        post.display_id.unwrap(),
        if let Some(image_url) = post.image_url {
            format!("<img src='{}' alt='Image' class='responsive-img'/>", image_url)
        } else {
            String::new()
        },
        post.content.replace("\n", "<br/>"),
        replies
    ))
}

fn initialize_database() {
    let conn = Connection::open("posts.db").unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS posts (
            id INTEGER PRIMARY KEY,
            content TEXT NOT NULL,
            parent_id INTEGER,
            reply_id INTEGER,
            display_id TEXT,
            timestamp INTEGER,
            image_url TEXT
        )",
        [],
    ).unwrap();
}

#[catch(413)]
fn payload_too_large() -> &'static str {
    "Payload too large! The file you are trying to upload exceeds the server's limit."
}

#[launch]
fn rocket() -> _ {
    initialize_database();
    rocket::build()
        .mount("/", routes![index, submit, submit_reply, reply])
        .mount("/static", FileServer::from(relative!("static")))
        .register("/", catchers![payload_too_large])
        .attach(AdHoc::on_liftoff("Config Logger", |_| {
            Box::pin(async move {
                let config = Config::figment();
                println!("Config: {:?}", config);
            })
        }))
}

