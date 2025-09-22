use anyhow::{Result, Context};
use bson::doc;
use chrono::{DateTime, Utc};
use goose::conversation::message::Message as GooseMessage;
use goose::conversation::Conversation;
use mongodb::{
    options::{ClientOptions, ServerApi, ServerApiVersion},
    Client, Collection, Database,
};
use serde::{Deserialize, Serialize};
use tracing::{info, error, debug, warn};
use uuid::Uuid;

// Correct schema based on actual MongoDB data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDocument {
    #[serde(rename = "_id")]
    pub id: String,  // _id is actually a string UUID in your data
    pub session_id: String,
    pub created_at: String,  // String timestamp, not DateTime
    pub updated_at: bson::Bson,  // Complex MongoDB date object - handle as raw BSON
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDocument {
    #[serde(rename = "_id")]
    pub id: String,  // String ID for messages too
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

pub struct DatabaseManager {
    pub client: Client,
    pub database: Database,
    pub sessions: Collection<SessionDocument>,
    pub messages: Collection<MessageDocument>,
    pub connection_url: String,
    pub db_name: String,
}

impl DatabaseManager {
    pub async fn new(connection_url: &str, db_name: &str) -> Result<Self> {
        info!("🔌 Connecting to MongoDB: {}", connection_url);
        info!("📦 Database: {}", db_name);
        
        let mut client_options = ClientOptions::parse(connection_url)
            .await
            .context("Failed to parse MongoDB connection URL")?;
        
        // Set server API version
        let server_api = ServerApi::builder().version(ServerApiVersion::V1).build();
        client_options.server_api = Some(server_api);
        
        // Create client
        let client = Client::with_options(client_options)
            .context("Failed to create MongoDB client")?;
        
        // Test connection with ping
        debug!("🏓 Testing MongoDB connection...");
        client
            .database("admin")
            .run_command(doc! {"ping": 1})
            .await
            .context("Failed to ping MongoDB server")?;
            
        info!("✅ Successfully connected to MongoDB!");
        
        let database = client.database(db_name);
        let sessions = database.collection::<SessionDocument>("sessions");
        let messages = database.collection::<MessageDocument>("messages");
        
        debug!("📊 Database collections initialized");
        
        Ok(Self {
            client,
            database,
            sessions,
            messages,
            connection_url: connection_url.to_string(),
            db_name: db_name.to_string(),
        })
    }
    
    pub async fn create_session(&self) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        
        debug!("➕ Creating session: {}", session_id);
        
        let session_doc = SessionDocument {
            id: session_id.clone(),  // Use same UUID for both _id and session_id
            session_id: session_id.clone(),
            created_at: now.to_rfc3339(),  // Store as RFC3339 string
            updated_at: bson::to_bson(&now).unwrap_or(bson::Bson::Null),  // Store as BSON timestamp
        };
        
        let insert_result = self.sessions
            .insert_one(&session_doc)
            .await
            .context("Failed to insert session into MongoDB")?;
            
        debug!("📝 Inserted session with _id: {}", insert_result.inserted_id);
        info!("✅ Created session in MongoDB: {}", session_id);
        Ok(session_id)
    }
    
    pub async fn get_session(&self, session_id: &str) -> Result<Option<SessionDocument>> {
        debug!("🔍 Looking up session: {}", session_id);
        
        let filter = doc! { "session_id": session_id };
        let result = self.sessions
            .find_one(filter)
            .await
            .context("Failed to query session from MongoDB")?;
            
        match &result {
            Some(session) => {
                debug!("✅ Found session: {} (created: {}, _id: {})", 
                       session_id, session.created_at, session.id);
            },
            None => debug!("❌ Session not found: {}", session_id),
        }
        
        Ok(result)
    }
    
    pub async fn delete_session(&self, session_id: &str) -> Result<bool> {
        info!("🗑️ Deleting session: {}", session_id);
        
        let filter = doc! { "session_id": session_id };
        
        // Delete all messages for this session first
        let message_result = self.messages
            .delete_many(filter.clone())
            .await
            .context("Failed to delete messages from MongoDB")?;
            
        debug!("🗑️ Deleted {} messages for session {}", message_result.deleted_count, session_id);
        
        // Delete the session
        let session_result = self.sessions
            .delete_one(filter)
            .await
            .context("Failed to delete session from MongoDB")?;
            
        let deleted = session_result.deleted_count > 0;
        if deleted {
            info!("✅ Deleted session: {}", session_id);
        } else {
            warn!("⚠️ Session not found for deletion: {}", session_id);
        }
        
        Ok(deleted)
    }
    
    pub async fn list_sessions(&self) -> Result<Vec<SessionDocument>> {
        debug!("📋 Listing all sessions from MongoDB");
        
        // This is the problematic line - let's debug it step by step
        debug!("🔍 Creating find cursor...");
        let mut cursor = self.sessions
            .find(doc! {})
            .sort(doc! { "created_at": -1 })
            .await
            .context("Failed to create cursor for sessions")?;
        
        debug!("🔍 Cursor created successfully, starting iteration...");
        let mut sessions = Vec::new();
        
        while cursor.advance().await.context("Failed to advance cursor")? {
            debug!("🔍 Attempting to deserialize document...");
            
            // Let's see the raw document first
            let raw_doc = cursor.current();
            debug!("📜 Raw document: {:?}", raw_doc);
            
            let session = cursor.deserialize_current()
                .context("Failed to deserialize session document")?;
            
            debug!("✅ Successfully deserialized session: {} (created: {})", 
                   session.session_id, session.created_at);
            sessions.push(session);
        }
            
        info!("📋 Successfully loaded {} sessions from MongoDB", sessions.len());
        Ok(sessions)
    }
    
    pub async fn add_message(&self, session_id: &str, message: &GooseMessage) -> Result<String> {
        debug!("💬 Adding message to session {}", session_id);
        
        let content = message.as_concat_text();
        let role = format!("{:?}", message.role);
        let message_id = Uuid::new_v4().to_string();
        
        debug!("💬 Message role: {}, content length: {}", role, content.len());
        
        let message_doc = MessageDocument {
            id: message_id.clone(),  // Use string ID to match session pattern
            session_id: session_id.to_string(),
            role,
            content,
            timestamp: Utc::now(),
        };
        
        let insert_result = self.messages
            .insert_one(&message_doc)
            .await
            .context("Failed to insert message into MongoDB")?;
        
        debug!("📝 Inserted message with _id: {}", insert_result.inserted_id);
        
        // Update session timestamp with BSON format
        let filter = doc! { "session_id": session_id };
        let update = doc! { "$set": { 
            "updated_at": bson::to_bson(&Utc::now()).unwrap_or(bson::Bson::Null)
        } };
        self.sessions
            .update_one(filter, update)
            .await
            .context("Failed to update session timestamp")?;
        
        debug!("✅ Added message {} to session {}", message_id, session_id);
        Ok(message_id)
    }
    
    pub async fn get_conversation(&self, session_id: &str) -> Result<Conversation> {
        debug!("📖 Loading conversation for session: {}", session_id);
        
        let filter = doc! { "session_id": session_id };
        let mut cursor = self.messages
            .find(filter)
            .sort(doc! { "timestamp": 1 })
            .await
            .context("Failed to create cursor for messages")?;
        
        let mut messages = Vec::new();
        
        while cursor.advance().await.context("Failed to advance cursor")? {
            let msg_doc = cursor.deserialize_current()
                .context("Failed to deserialize message document")?;
            
            debug!("📜 Message: {} - {} ({} chars)", 
                   msg_doc.role, msg_doc.timestamp, msg_doc.content.len());
            
            let message = match msg_doc.role.as_str() {
                "User" => GooseMessage::user().with_text(&msg_doc.content),
                "Assistant" => GooseMessage::assistant().with_text(&msg_doc.content),
                _ => {
                    warn!("⚠️ Unknown message role: {}, defaulting to user", msg_doc.role);
                    GooseMessage::user().with_text(&msg_doc.content)
                }
            };
            
            messages.push(message);
        }
        
        info!("📖 Loaded {} messages for session {}", messages.len(), session_id);
        Ok(Conversation::new_unvalidated(messages))
    }
    
    pub async fn get_message_count(&self, session_id: &str) -> Result<usize> {
        debug!("🔢 Counting messages for session: {}", session_id);
        
        let filter = doc! { "session_id": session_id };
        let count = self.messages
            .count_documents(filter)
            .await
            .context("Failed to count messages in MongoDB")?;
            
        debug!("🔢 Session {} has {} messages", session_id, count);
        Ok(count as usize)
    }
    
    pub async fn update_conversation(&self, session_id: &str, conversation: &Conversation) -> Result<()> {
        info!("🔄 Updating conversation for session: {}", session_id);
        
        let filter = doc! { "session_id": session_id };
        let delete_result = self.messages
            .delete_many(filter)
            .await
            .context("Failed to delete existing messages")?;
            
        debug!("🗑️ Deleted {} existing messages", delete_result.deleted_count);
        
        for (i, message) in conversation.messages().iter().enumerate() {
            debug!("💾 Saving message {}/{}", i + 1, conversation.messages().len());
            self.add_message(session_id, message).await
                .context("Failed to add message during conversation update")?;
        }
        
        info!("✅ Updated conversation for session {} with {} messages", 
              session_id, conversation.messages().len());
        Ok(())
    }
    
    pub async fn health_check(&self) -> bool {
        debug!("❤️ Checking MongoDB health...");
        
        match self.client.database("admin").run_command(doc! {"ping": 1}).await {
            Ok(_) => {
                debug!("✅ MongoDB ping successful");
                true
            },
            Err(e) => {
                error!("❌ MongoDB health check failed: {}", e);
                false
            }
        }
    }
}
