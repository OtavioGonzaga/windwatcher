use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub id: Uuid,
    pub username: String,
    pub roles: Vec<String>,
}
