
#[macro_use] extern crate rocket;

use rocket::fs::{FileServer, relative, TempFile};
use rocket::form::Form;
use rocket::response::content::RawHtml;
use rocket::response::Redirect;
use rocket::serde::{Serialize, Deserialize};
use rusqlite::{params, Connection};
use rand::{distributions::Alphanumeric, Rng};
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::PathBuf;
use std::fs::{self, create_dir_all};

#[derive(Debug, Serialize, Deserialize, FromForm)]
struct Post {
    id: Option<i32>,
    content: String,
    parent_id: Option<i32>,
    reply_id: Option<i32>,
    display_id: Option<String>,
    timestamp: Option<u64>,
    file_path: Option<String>,
}

#[derive(FromForm)]
struct PostForm<'r> {
    content: &'r str,
    file: Option<TempFile<'r>>,
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

#[post("/submit", data = "<post_form>")]
async fn submit(mut post_form: Form<PostForm<'_>>) -> Redirect {
    let content = post_form.content.to_string();
    let display_id = generate_display_id();
    let timestamp = current_timestamp();

    let file_path = if let Some(file) = post_form.file.as_mut() {
        if let Some(file_name) = file.name() {
            if !file_name.is_empty() {
                let allowed_extensions = ["jpg", "png", "bmp", "gif", "webp", "webm", "mp4", "mp3"];
                let extension = file_name.split('.').last().unwrap_or("").to_lowercase();
                println!("Processing file with name: {}", file_name); // Debug log
                println!("Extracted file extension: {}", extension); // Debug log
                println!("File size: {}", file.len()); // Debug log

                if allowed_extensions.contains(&extension.as_str()) && file.len() <= 20 * 1024 * 1024 {
                    let upload_dir = "static/uploads";
                    if let Err(e) = create_dir_all(upload_dir) {
                        eprintln!("Failed to create upload directory: {:?}", e);
                        return Redirect::to("/");
                    }

                    let file_path = format!("{}/{}", upload_dir, file_name);
                    let path: PathBuf = file_path.clone().into();
                    match file.persist_to(&path).await {
                        Ok(_) => {
                            println!("File uploaded to: {}", file_path);
                            Some(file_path)
                        }
                        Err(e) => {
                            eprintln!("Failed to persist file: {:?}", e);
                            None
                        }
                    }
                } else {
                    eprintln!("File extension not allowed or file too large.");
                    None
                }
            } else {
                eprintln!("File name is empty.");
                None
            }
        } else {
            eprintln!("File name is None.");
            None
        }
    } else {
        eprintln!("No file provided.");
        None
    };

    let conn = Connection::open("posts.db").unwrap();
    conn.execute(
        "INSERT INTO posts (content, parent_id, reply_id, display_id, timestamp, file_path) VALUES (?1, NULL, NULL, ?2, ?3, ?4)",
        params![content, display_id, timestamp, file_path],
    ).unwrap();

    Redirect::to("/")
}

#[post("/submit_reply/<parent_id>", data = "<post_form>")]
async fn submit_reply(parent_id: i32, post_form: Form<PostForm<'_>>) -> Redirect {
    let content = post_form.content.to_string();
    let timestamp = current_timestamp();

    let conn = Connection::open("posts.db").unwrap();
    let reply_id: i32 = conn.query_row(
        "SELECT COALESCE(MAX(reply_id), 0) + 1 FROM posts WHERE parent_id = ?1",
        params![parent_id],
        |row| row.get(0)
    ).unwrap();

    conn.execute(
        "INSERT INTO posts (content, parent_id, reply_id, display_id, timestamp, file_path) VALUES (?1, ?2, ?3, NULL, ?4, NULL)",
        params![content, parent_id, reply_id, timestamp],
    ).unwrap();

    // Update the timestamp of the original post to bring it to the top
    conn.execute(
        "UPDATE posts SET timestamp = ?1 WHERE id = ?2",
        params![timestamp, parent_id],
    ).unwrap();

    Redirect::to(format!("/reply/{}", parent_id))
}

#[get("/?<page>")]
fn index(page: Option<usize>) -> RawHtml<String> {
    let page = page.unwrap_or(1);
    let posts_per_page = 10;
    let offset = (page - 1) * posts_per_page;

    let conn = Connection::open("posts.db").unwrap();
    let mut stmt = conn.prepare("SELECT id, content, display_id, file_path FROM posts WHERE parent_id IS NULL ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2").unwrap();
    let post_iter = stmt.query_map(params![posts_per_page as i64, offset as i64], |row| {
        Ok(Post {
            id: row.get(0)?,
            content: row.get(1)?,
            parent_id: None,
            reply_id: None,
            display_id: row.get(2)?,
            timestamp: None,
            file_path: row.get(3)?,
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
                <p><b>{}</b>: {}</p>
                {} 
                <a href='/reply/{}' class='reply-button'>Reply ({})</a>
            </div>",
            post.display_id.as_ref().unwrap(), 
            post.content, 
            if let Some(file_path) = post.file_path.as_ref() {
                let extension = file_path.split('.').last().unwrap_or("");
                if ["jpg", "png", "bmp", "gif", "webp"].contains(&extension) {
                    format!("<img src='/{}' class='post-image'/><br/>", file_path)
                } else if ["webm", "mp4"].contains(&extension) {
                    format!("<video controls><source src='/{}' type='video/{}'></video><br/>", file_path, extension)
                } else if extension == "mp3" {
                    format!("<audio controls><source src='/{}' type='audio/mpeg'></audio><br/>", file_path)
                } else {
                    String::new()
                }
            } else {
                String::new()
            },
            post.id.unwrap(), reply_count
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
                <link rel="icon" type="image/gif" href="/static/favicon.gif">
            </head>
            <body>
                <div class="container">
                    <form action="/submit" method="post" enctype="multipart/form-data">
                        <textarea name="content" required></textarea><br/>
                        <input type="file" name="file"><br/>
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
    
    let mut stmt = conn.prepare("SELECT id, content, display_id, file_path FROM posts WHERE id = ?1").unwrap();
    let post = stmt.query_row(params![post_id], |row| {
        Ok(Post {
            id: row.get(0)?,
            content: row.get(1)?,
            parent_id: None,
            reply_id: None,
            display_id: row.get(2)?,
            timestamp: None,
            file_path: row.get(3)?,
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
            file_path: None,
        })
    }).unwrap();

    let mut replies = String::new();
    for reply in reply_iter {
        let reply = reply.unwrap();
        replies.push_str(&format!(
            "<div class='post'>
                <p><b>Reply {}</b>: {}</p>
            </div>",
            reply.reply_id.unwrap(), reply.content
        ));
    }

    RawHtml(format!(
        r#"
        <html>
            <head>
                <link rel="stylesheet" type="text/css" href="/static/styles.css">
                <link rel="icon" type="image/gif" href="/static/favicon.gif">
            </head>
            <body>
                <div class="container">
                    <a href="/" class="home-button">Home</a>
                    <form action="/submit_reply/{}" method="post">
                        <textarea name="content" required></textarea><br/>
                        <input type="submit" value="Reply" class="button">
                    </form>
                    <div class="post">
                        <p><b>{}</b>: {}</p>
                        {}
                    </div>
                    <div class="replies">{}</div>
                </div>
            </body>
        </html>
        "#,
        post_id,
        post.display_id.unwrap(), post.content,
        if let Some(file_path) = post.file_path {
            let extension = file_path.split('.').last().unwrap_or("");
            if ["jpg", "png", "bmp", "gif", "webp"].contains(&extension) {
                format!("<img src='/{}' class='post-image'/><br/>", file_path)
            } else if ["webm", "mp4"].contains(&extension) {
                format!("<video controls><source src='/{}' type='video/{}'></video><br/>", file_path, extension)
            } else if extension == "mp3" {
                format!("<audio controls><source src='/{}' type='audio/mpeg'></audio><br/>", file_path)
            } else {
                String::new()
            }
        } else {
            String::new()
        },
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
            file_path TEXT
        )",
        [],
    ).unwrap();
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_parent_id ON posts (parent_id)",
        [],
    ).unwrap();
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_timestamp ON posts (timestamp)",
        [],
    ).unwrap();
}

#[launch]
fn rocket() -> _ {
    initialize_database();
    rocket::build()
        .mount("/", routes![index, submit, submit_reply, reply])
        .mount("/static", FileServer::from(relative!("static")))
}
