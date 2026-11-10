// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Errors every transport reports the same way: a code and a message,
//! and, for a failure the app tells people about in their own language, a
//! reason and its values (M8).

use oikome_core::CatalogError;
use oikome_engine::EngineError;
use oikome_node::access_log::AccessLogError;
use oikome_node::SessionError;
use oikome_semantic::SemanticError;
use serde::Serialize;
use std::collections::BTreeMap;
use ts_rs::TS;

/// What kind of failure an [`ApiError`] is. Transports map it to their own
/// status (HTTP: 400, 401, 403, 404, 409, 500).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request is malformed or names something it may not.
    Invalid,
    /// The caller is not logged in.
    Unauthenticated,
    /// The caller may not do this.
    PermissionDenied,
    /// What the request names does not exist.
    NotFound,
    /// It already exists.
    Conflict,
    /// The node failed; details are in its log, not in the message.
    Internal,
}

/// A failed call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    pub code: ErrorCode,
    /// In English, for the log, the command line and the SQL workbench.
    pub message: String,
    /// Which of the failures written for people this is
    /// (`invite_ended`), so an app words it in its own language; the
    /// message is the fallback for a reason it doesn't know.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    /// The values the reason's sentence names (`name`, `path`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    #[ts(as = "Option<BTreeMap<String, String>>", optional)]
    pub params: BTreeMap<String, String>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: None,
            params: BTreeMap::new(),
        }
    }

    /// Name the failure for people, `invite_ended`, so apps can word it.
    pub fn because(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_string());
        self
    }

    /// A value the reason's sentence names.
    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_string(), value.into());
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Invalid, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PermissionDenied, message)
    }

    /// An internal failure: logged with its details, reported without them.
    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::warn!(error = %error, "api call failed");
        Self::new(
            ErrorCode::Internal,
            "the node could not complete the request",
        )
    }
}

impl From<CatalogError> for ApiError {
    fn from(e: CatalogError) -> Self {
        match e {
            CatalogError::SpaceNotFound(_)
            | CatalogError::TableNotFound(_)
            | CatalogError::LocationNotFound(_)
            | CatalogError::ViewNotFound(_)
            | CatalogError::PolicyNotFound(_)
            | CatalogError::ContactNotFound(_)
            | CatalogError::StorageNotFound(_) => Self::not_found(e.to_string()),
            CatalogError::SpaceExists(_)
            | CatalogError::ViewExists(_)
            | CatalogError::PolicyExists(_)
            | CatalogError::LocationHasWriter { .. } => {
                Self::new(ErrorCode::Conflict, e.to_string())
            }
            CatalogError::InvalidProperty(_)
            | CatalogError::KindFormatMismatch { .. }
            | CatalogError::IncompatibleFormat { .. } => Self::invalid(e.to_string()),
            other => Self::internal(other),
        }
    }
}

impl From<SessionError> for ApiError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Catalog(e) => e.into(),
            SessionError::PermissionDenied(_) => Self::permission_denied(e.to_string()),
            SessionError::AlreadyExists(_) => Self::new(ErrorCode::Conflict, e.to_string()),
            SessionError::Semantic(SemanticError::TableNotFound(_))
            | SessionError::Engine(EngineError::TableNotFound(_)) => Self::not_found(e.to_string()),
            // The owner's own statements: what went wrong is useful to them.
            SessionError::Semantic(_)
            | SessionError::Unsupported(_)
            | SessionError::NoDefaultSpace
            | SessionError::TransactionOpen(_)
            | SessionError::NoTransaction
            | SessionError::MultiSpaceNotSupported(_)
            | SessionError::ReadOnlySpace(_)
            | SessionError::PublishedSpace { .. }
            | SessionError::EngineNotRegistered(_)
            | SessionError::Identity(_)
            | SessionError::Engine(
                EngineError::Sql(_)
                | EngineError::TypeMismatch { .. }
                | EngineError::Unsupported(..)
                | EngineError::ExtensionUnavailable(..)
                | EngineError::Transaction(_),
            ) => Self::invalid(e.to_string()),
            other => Self::internal(other),
        }
    }
}

impl From<AccessLogError> for ApiError {
    fn from(e: AccessLogError) -> Self {
        Self::internal(e)
    }
}
