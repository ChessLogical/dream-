use actix_files as fs;
use actix_multipart::Multipart;
use actix_web::{web, App, HttpResponse, HttpServer, Result};
use futures_util::stream::StreamExt as _;
use std::collections::HashMap;
use std::fs::read_to_string;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::sync::Mutex;
use actix_web::web::Data;
use rusqlite::{params, Connection, Result as SqlResult};
use rand::{distributions::Alphanumeric, Rng};
use std::collections::hash_map::DefaultHasher;

// Maximum file size (20 MB)
const MAX_SIZE: usize = 20 * 1024 * 1024;
const POSTS_PER_PAGE: usize = 30;

fn render_template(path: &str, context: &HashMap<&str, String>) -> String {
    let template = read_to_string(path).expect("Unable to read template file");
    let mut rendered = template;
    for (key, value) in context {
        let placeholder = format!("{{{{{}}}}}", key);
        rendered = rendered.replace(&placeholder, value);
    }
    rendered
}

fn generate_color_from_id(id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let hash = hasher.finish();
    let r = (hash & 0xFF) as u8;
    let g = ((hash >> 8) & 0xFF) as u8;
    let b = ((hash >> 16) & 0xFF) as u8;
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

async fn save_file(mut payload: Multipart, conn: web::Data<Mutex<Connection>>) -> Result<HttpResponse> {
    let mut title = String::new();
    let mut message = String::new();
    let mut file_path = None;
    let mut parent_id: i32 = 0;

    while let Some(item) = payload.next().await {
        let mut field = item?;
        let content_disposition = field.content_disposition().clone();
        let name = content_disposition.get_name().unwrap_or("").to_string();

        match name.as_str() {
            "title" => {
                while let Some(chunk) = field.next().await {
                    let data = chunk?;
                    title.push_str(&String::from_utf8_lossy(&data));
                }
            },
            "message" => {
                while let Some(chunk) = field.next().await {
                    let data = chunk?;
                    message.push_str(&String::from_utf8_lossy(&data));
                }
            },
            "file" => {
                if let Some(filename) = content_disposition.get_filename() {
                    let file_extension = filename.split('.').last().unwrap_or("");
                    let sanitized_filename = sanitize_filename::sanitize(&filename);
                    let unique_id: String = rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(6)
                        .map(char::from)
                        .collect();
                    let unique_filename = format!("{}-{}", unique_id, sanitized_filename);

                    let valid_image_extensions = ["jpg", "jpeg", "png", "gif", "webp"];
                    let valid_video_extensions = ["mp4", "mp3", "webm"];

                    if valid_image_extensions.contains(&file_extension) || valid_video_extensions.contains(&file_extension) {
                        let file_path_string = format!("./static/{}", unique_filename);
                        let file_path_clone = file_path_string.clone();
                        let mut f = web::block(move || std::fs::File::create(file_path_clone)).await??;

                        while let Some(chunk) = field.next().await {
                            let data = chunk?;
                            f = web::block(move || f.write_all(&data).map(|_| f)).await??;
                        }

                        file_path = Some(file_path_string);
                    }
                }
            },
            "parent_id" => {
                while let Some(chunk) = field.next().await {
                    let data = chunk?;
                    parent_id = String::from_utf8_lossy(&data).trim().parse().unwrap_or(0);
                }
            },
            _ => {},
        }
    }

    if title.trim().is_empty() || message.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().body("Title and message are mandatory."));
    }

    if title.len() > 30 || message.len() > 50000 {
        return Ok(HttpResponse::BadRequest().body("Title or message is too long."));
    }

    let post_id: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();

    let conn = conn.lock().unwrap();
    conn.execute(
        "INSERT INTO files (post_id, parent_id, title, message, file_path) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![post_id, parent_id, title, message, file_path],
    ).unwrap();

    if parent_id != 0 {
        conn.execute(
            "UPDATE files SET last_reply_at = CURRENT_TIMESTAMP WHERE id = ?1 OR parent_id = ?1",
            params![parent_id],
        ).unwrap();
    }

    if parent_id == 0 {
        Ok(HttpResponse::SeeOther().append_header(("Location", "/")).finish())
    } else {
        Ok(HttpResponse::SeeOther().append_header(("Location", format!("/post/{}", parent_id))).finish())
    }
}

async fn view_post(conn: web::Data<Mutex<Connection>>, path: web::Path<i32>) -> Result<HttpResponse> {
    let conn = conn.lock().unwrap();
    let post_id = path.into_inner();

    let mut stmt = conn.prepare("SELECT id, post_id, parent_id, title, message, file_path FROM files WHERE id = ?1 OR parent_id = ?1 ORDER BY id ASC").unwrap();
    let posts = stmt.query_map(params![post_id], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i32>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    }).unwrap();

    let mut posts_html = String::new();
    let mut is_original_post = true;
    let mut reply_count = 1;

    for post in posts {
        let (_id, _post_id, _parent_id, title, message, file_path) = post.unwrap();
        posts_html.push_str("<div class=\"post\">");
        if is_original_post {
            posts_html.push_str("<div class=\"post-id\">Original Post</div>");
            is_original_post = false;
        } else {
            posts_html.push_str(&format!("<div class=\"post-id\">Reply {}</div>", reply_count));
            reply_count += 1;
        }
        posts_html.push_str(&format!("<div class=\"post-title\">{}</div>", title));
        if let Some(file_path) = file_path {
            if file_path.ends_with(".jpg") || file_path.ends_with(".jpeg") || file_path.ends_with(".png") || file_path.ends_with(".gif") || file_path.ends_with(".webp") {
                posts_html.push_str(&format!(r#"<img src="/static/{}"><br>"#, file_path.trim_start_matches("./static/")));
            } else if file_path.ends_with(".mp4") || file_path.ends_with(".mp3") || file_path.ends_with(".webm") {
                posts_html.push_str(&format!(r#"<video controls><source src="/static/{}"></video><br>"#, file_path.trim_start_matches("./static/")));
            }
        }
        posts_html.push_str(&format!("<div class=\"post-message\">{}</div>", message));
        posts_html.push_str("</div>");
    }

    let context = HashMap::from([
        ("PARENT_ID", post_id.to_string()),
        ("POSTS", posts_html),
    ]);

    let body = render_template("templates/view_post.html", &context);

    Ok(HttpResponse::Ok().content_type("text/html").body(body))
}

async fn index(conn: web::Data<Mutex<Connection>>, query: web::Query<HashMap<String, String>>) -> Result<HttpResponse> {
    let conn = conn.lock().unwrap();
    let page: usize = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let offset = (page - 1) * POSTS_PER_PAGE;

    let mut stmt = conn.prepare("SELECT id, post_id, title, message, file_path FROM files WHERE parent_id = 0 ORDER BY last_reply_at DESC LIMIT ?1 OFFSET ?2").unwrap();
    let posts = stmt.query_map(params![POSTS_PER_PAGE as i64, offset as i64], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    }).unwrap();

    let mut posts_html = String::new();

    for post in posts {
        let (id, post_id, title, message, file_path) = post.unwrap();

        let reply_count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE parent_id = ?1",
            params![id],
            |row| row.get(0),
        ).unwrap_or(0);

        let truncated_message = if message.len() > 2700 {
            format!("{}... <a href=\"/post/{}\" class=\"view-full-post\">Click here to open full post</a>", &message[..2700], id)
        } else {
            message.clone()
        };

        let post_color = generate_color_from_id(&post_id);

        posts_html.push_str("<div class=\"post\">");
        posts_html.push_str(&format!("<div class=\"post-id-box\" style=\"background-color: {}\">{}</div>", post_color, post_id));
        posts_html.push_str(&format!("<div class=\"post-title title-green\">{}</div>", title));
        if let Some(file_path) = file_path {
            if file_path.ends_with(".jpg") || file_path.ends_with(".jpeg") || file_path.ends_with(".png") || file_path.ends_with(".gif") || file_path.ends_with(".webp") {
                posts_html.push_str(&format!(r#"<img src="/static/{}"><br>"#, file_path.trim_start_matches("./static/")));
            } else if file_path.ends_with(".mp4") || file_path.ends_with(".mp3") || file_path.ends_with(".webm") {
                posts_html.push_str(&format!(r#"<video controls><source src="/static/{}"></video><br>"#, file_path.trim_start_matches("./static/")));
            }
        }
        posts_html.push_str(&format!("<div class=\"post-message\">{}</div>", truncated_message));
        posts_html.push_str(&format!("<a class=\"reply-button\" href=\"/post/{}\">Reply ({})</a>", id, reply_count));
        posts_html.push_str("</div>");
    }

    let next_page = page + 1;
    let prev_page = if page > 1 { page - 1 } else { 1 };
    let mut pagination_html = String::new();
    if page > 1 {
        pagination_html.push_str(&format!(r#"<a href="/?page={}">Previous</a>"#, prev_page));
    }
    pagination_html.push_str(&format!(r#"<a href="/?page={}">Next</a>"#, next_page));

    let context = HashMap::from([
        ("POSTS", posts_html),
        ("PAGINATION", pagination_html),
    ]);

    let body = render_template("templates/index.html", &context);

    Ok(HttpResponse::Ok().content_type("text/html").body(body))
}

fn initialize_db() -> SqlResult<Connection> {
    let conn = Connection::open("my_database.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            post_id TEXT NOT NULL,
            parent_id INTEGER,
            title TEXT NOT NULL,
            message TEXT NOT NULL,
            file_path TEXT,
            last_reply_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;
    Ok(conn)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let conn = initialize_db().unwrap();
    let conn_data = Data::new(Mutex::new(conn));

    HttpServer::new(move || {
        App::new()
            .app_data(conn_data.clone())
            .app_data(Data::new(web::JsonConfig::default().limit(MAX_SIZE)))
            .service(
                web::resource("/")
                    .route(web::get().to(index))
            )
            .service(
                web::resource("/upload")
                    .route(web::post().to(save_file))
            )
            .service(
                web::resource("/post/{id}")
                    .route(web::get().to(view_post))
            )
            .service(fs::Files::new("/static", "./static").show_files_listing())
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
