#[path="../src/modules/users/model.rs"] mod model;
#[path="../src/modules/users/repository.rs"] mod repository;
use mach::repository;
use model::{User,UserInput};
use repository::UserRepository;

#[repository(User)]
trait Queries {
    async fn count_by_phone_is_null_and_id_greater_than(threshold:i64)->Result<i64>;
    async fn exists_by_full_name_and_id_less_than(name:&str,limit:i64)->Result<bool>;
    async fn delete_by_email(email:&str)->Result<u64>;
}

#[mach::tokio::test(crate="mach::tokio")]
async fn generated_queries_and_transaction(){
    let db=mach::db::connect(&std::env::var("DATABASE_URL").unwrap(),2).await.unwrap();
    let original=User::find_or_fail(&db,41).await.unwrap();
    assert_eq!(original.full_name,"Stored before rename");
    assert!(original.phone.is_none());
    let mut tx=db.begin().await.unwrap();
    let data=UserInput{email:"o'brien@example.org".into(),full_name:"Test writer".into()};
    let user=User::create(&mut *tx,data).await.unwrap();
    assert_eq!(UserRepository::find_by_email(&mut *tx,"o'brien@example.org").await.unwrap().unwrap().id,user.id);
    assert!(Queries::exists_by_full_name_and_id_less_than(&mut *tx,"Test writer",100).await.unwrap());
    assert_eq!(Queries::count_by_phone_is_null_and_id_greater_than(&mut *tx,0).await.unwrap(),2);
    assert_eq!(Queries::delete_by_email(&mut *tx,"historic@example.org").await.unwrap(),1);
    tx.rollback().await.unwrap();
    assert!(User::find(&db,user.id).await.unwrap().is_none());
    assert!(User::find(&db,41).await.unwrap().is_some());
}
