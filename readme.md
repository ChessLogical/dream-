File Upload Board with Actix Web

This is a simple file upload board implemented using Actix Web in Rust. The application allows users to upload image and video files, which are then displayed on the main page. The files are sorted by their modification time, with the latest files appearing at the top.




![Screenshot 2024-05-27 064820](https://github.com/ChessLogical/actix/assets/169053333/8a81e8f7-7dc6-48a5-9d4d-28ab2add862c)



Features
File Upload: Supports uploading multiple files at once.
File Types: Accepts image files (JPG, JPEG, PNG, GIF, WEBP) and video files (MP4, MP3, WEBM).
File Size Limit: Restricts file uploads to a maximum size of 20MB.
Dark Theme: The application uses a modern dark-themed CSS for styling.
File Display: Displays uploaded files at half their original size with a thick line separating each post.

Prerequisites
Rust: Ensure that Rust is installed on your system. You can download it from rust-lang.org.
Cargo: Cargo is the Rust package manager and is included with Rust.
