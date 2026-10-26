#![warn(missing_docs)]

//! Embeddable messaging engine for Rust applications.

mod authentication;
mod conversation;
mod conversation_store;
mod conversations;
mod cursor;
mod id;
mod idempotency;
mod identity;
mod identity_store;
mod local_users;
mod message;
mod message_store;
mod messages;
mod pagination;
mod password;
mod queries;
mod read_models;
mod session;
#[cfg(feature = "sqlite")]
mod storage;
mod users;

pub use authentication::{AuthenticateLocalUser, AuthenticateLocalUserError};
pub use conversation::{
    Conversation, ConversationError, ConversationKind, ConversationMember, GroupRole, GroupTitle,
    GroupTitleError, MembershipRole,
};
pub use conversation_store::{
    AddMemberStoreOutcome, ChangeStoreOutcome, ConversationStore, ConversationStoreError,
    LeaveStoreOutcome, StartDirectStoreOutcome,
};
pub use conversations::{
    AddGroupMember, AddGroupMemberError, AddGroupMemberOutcome, CreateGroupConversation,
    CreateGroupConversationError, GetConversation, GetConversationError, LeaveGroupConversation,
    LeaveGroupConversationError, LeaveGroupOutcome, MutationOutcome, RemoveGroupMember,
    RemoveGroupMemberError, RenameGroupConversation, RenameGroupConversationError,
    SetGroupMemberRole, SetGroupMemberRoleError, StartDirectConversation,
    StartDirectConversationError, StartDirectConversationOutcome, TransferGroupOwnership,
    TransferGroupOwnershipError,
};
pub use cursor::CursorError;
pub use idempotency::{
    ConversationCreateCommand, ConversationCreateRequest, CreateCommandError, CreateCommandStore,
    CreateConversation, CreateDisposition, CreateResult, IdempotencyKey, IdempotencyKeyError,
    SendMessageCommand, message_request_fingerprint,
};
pub use identity_store::{IdentityStore, IdentityStoreError, LocalAuthentication};
pub use local_users::{
    CreateLocalUser, CreateLocalUserError, DisableUser, EnableUser, ManageUserError,
    RegisterLocalUser, RegisterLocalUserError, RegistrationPolicy,
};
pub use message::{
    Message, MessageContent, MessageContentError, MessageError, MessagePolicy, MessagePolicyError,
};
pub use message_store::{
    DeleteMessageStoreOutcome, EditMessageStoreOutcome, GetMessageStoreOutcome,
    ListMessagesStoreOutcome, MarkReadStoreOutcome, MessagePageSlice, MessageStore,
    MessageStoreError, SendMessageStoreOutcome, UnreadCountStoreOutcome,
};
pub use messages::{
    DeleteMessage, DeleteMessageError, DeleteMessageOutcome, EditMessage, EditMessageError,
    EditMessageOutcome, GetMessage, GetMessageError, GetUnreadCount, GetUnreadCountError,
    ListMessages, ListMessagesError, MarkConversationReadError, MarkConversationReadThrough,
    MarkReadOutcome, SendMessage, SendMessageError,
};
pub use pagination::{Page, PageLimit, PageLimitError, PageRequest};
pub use password::{
    Argon2idPasswordHasher, PasswordHash, PasswordHashError, PasswordHasher, PasswordPolicyError,
    SecretPassword,
};
pub use queries::{
    ConversationQueryOutcome, FindUsers, GetConversationSummary, ListConversationMembers,
    ListConversations, MessagingQueryError, MessagingQueryStore, QuerySlice, QueryStoreError,
};
pub use read_models::{
    ConversationBoundary, ConversationMemberSummary, ConversationReadState, ConversationSummary,
    MemberBoundary, MessageView, ReplyPreview, UserSummary,
};
pub use session::{
    AccessToken, AccessTokenClaims, AccessTokenCodec, AccessTokenError, CreateSession,
    CreateSessionError, OsRefreshTokenProvider, RefreshRotationResult, RefreshSession,
    RefreshSessionError, RefreshToken, RefreshTokenError, RefreshTokenHash, RefreshTokenProvider,
    RevokeSession, RevokeSessionError, Session, SessionError, SessionGrant, SessionPolicy,
    SessionPolicyError, SessionStore, SessionStoreError,
};
#[cfg(feature = "jwt-hs256")]
pub use session::{Hs256JwtAccessTokenCodec, Hs256SigningSecret, Hs256SigningSecretError};

pub use id::{ConversationId, IdentifierError, MessageId, SessionId, UserId};
pub use identity::{User, Username, UsernameError};
#[cfg(feature = "sqlite")]
pub use storage::{MigrationStatus, SqliteStorage, SqliteStorageError, SqliteStorageOptions};
pub use users::{GetUser, GetUserError};
