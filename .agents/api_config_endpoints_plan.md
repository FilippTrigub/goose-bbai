# API Configuration Endpoints Implementation Plan

## Overview
Add REST API endpoints for extension management and settings configuration to the existing Goose API server, mirroring the functionality from `configure.rs`.

## Current API Structure Analysis
- Uses Axum framework with JSON responses
- AppState contains: Agent, Provider, DatabaseManager
- Routes pattern: `/api/v1/{resource}`
- Error handling with proper HTTP status codes
- CORS enabled for web clients

## New Endpoints to Add

### 1. Extension Management Routes
```
GET    /api/v1/extensions              # List all extensions with status
POST   /api/v1/extensions              # Add new extension
PUT    /api/v1/extensions/{name}/toggle # Enable/disable extension
DELETE /api/v1/extensions/{name}       # Remove extension (if disabled)
```

### 2. Settings Management Routes
```
GET    /api/v1/settings/mode           # Get current Goose mode
PUT    /api/v1/settings/mode           # Update Goose mode
GET    /api/v1/settings/output         # Get tool output level
PUT    /api/v1/settings/output         # Update tool output level
GET    /api/v1/settings/max-turns      # Get max turns setting
PUT    /api/v1/settings/max-turns      # Update max turns
GET    /api/v1/settings/experiments    # List experiments with status
PUT    /api/v1/settings/experiments    # Toggle experiments
```

## Implementation Steps

### Phase 1: Data Structures
Add response/request structs:
- `ExtensionInfo`, `ExtensionListResponse`
- `ExtensionCreateRequest`, `ExtensionUpdateRequest`
- `SettingsResponse` for various settings types

### Phase 2: Extension Endpoints
- Use `ExtensionConfigManager` from goose::config
- Handle built-in vs external extensions
- Validate extension configurations
- Update AppState to reload agent extensions

### Phase 3: Settings Endpoints
- Use `Config::global()` for reading/writing settings
- Support GOOSE_MODE, GOOSE_CLI_MIN_PRIORITY, GOOSE_MAX_TURNS
- Handle ExperimentManager for experiments

### Phase 4: Integration
- Add routes to main router
- Update endpoint count in startup logs
- Add test commands to API_TEST_COMMANDS.md

## Key Considerations
- **No Auth/Provider routes** - Only extension and settings management
- **Read-only agent state** - Extensions require server restart to reload
- **Environment variable precedence** - API should respect env vars over config
- **Error handling** - Proper HTTP status codes and error messages
- **Simple design** - Mirror CLI functionality without complex validation

## Dependencies
- `goose::config::{Config, ExtensionConfigManager, ExperimentManager}`
- `goose::config::extensions::name_to_key`
- `goose::config::permission::PermissionManager`