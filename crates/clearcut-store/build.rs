// Written by Kone Tshivhinda & Claude
// `sqlx::migrate!` embeds the migration files at compile time. Without this,
// Cargo would not notice a changed .sql file and would keep using the old SQL.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
