#[mach::tokio::test(crate="mach::tokio")]
async fn seed_history(){
    let db=mach::db::connect(&std::env::var("DATABASE_URL").unwrap(),1).await.unwrap();
    mach::sqlx::query("INSERT INTO users (id,email,name) VALUES (41,$1,$2)").bind("historic@example.org").bind("Stored before rename").execute(&db).await.unwrap();
}
