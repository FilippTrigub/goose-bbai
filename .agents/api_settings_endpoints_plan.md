# API Settings Management Implementation Plan

## Overview
Add REST API endpoints for Goose settings configuration, mirroring the functionality from `configure.rs` settings dialogs. Focus on core runtime settings that don't require authentication/provider changes.

## Settings to Support

### Target Configuration Keys
Based on documentation and `configure.rs` analysis:

1. **GOOSE_MODEL** - Current LLM model (string)
2. **GOOSE_TEMPERATURE** - Model temperature 0.0-2.0 (float)  
3. **GOOSE_MODE** - Agent behavior mode (string: auto/approve/smart_approve/chat)
4. **GOOSE_MAX_TURNS** - Max consecutive actions (integer, default: 1000)
5. **GOOSE_CLI_MIN_PRIORITY** - Tool output filter (float: 0.0/0.2/0.8)
6. **GOOSE_RECIPE_GITHUB_REPO** - Recipe source repo (string: owner/repo)
7. **GOOSE_AUTO_COMPACT_THRESHOLD** - Context management (integer)

## Implementation Analysis

### Current Patterns from configure.rs
- Uses `Config::global()` for reading/writing
- `config.set_param(key, Value::String/Number)` for updates
- `config.get_param(key)` for reading with fallback defaults
- Environment variable precedence checking
- Validation for input values

### Configuration Storage
- Uses `Config::global()` which reads/writes to YAML config file
- Some settings use `config.set_param()` for non-secret values
- Values stored as `serde_json::Value` (String, Number, Bool)

## API Endpoints Design

### 1. Settings Management Routes
```
GET    /api/v1/settings              # Get all configurable settings
GET    /api/v1/settings/{key}        # Get specific setting value
PUT    /api/v1/settings/{key}        # Update specific setting value
DELETE /api/v1/settings/{key}        # Reset setting to default
```

### 2. Bulk Settings Route
```
PUT    /api/v1/settings              # Update multiple settings at once
```

## Data Structures

### Request/Response Types
```rust
#[derive(Serialize)]
struct SettingInfo {
    key: String,
    value: Option<serde_json::Value>,
    default_value: Option<serde_json::Value>,
    description: String,
    value_type: String,  // "string", "number", "boolean"
    validation: Option<SettingValidation>,
    env_override: Option<String>,
}

#[derive(Serialize)]
struct SettingValidation {
    min: Option<f64>,
    max: Option<f64>,
    allowed_values: Option<Vec<String>>,
    pattern: Option<String>,
}

#[derive(Deserialize)]
struct SettingUpdateRequest {
    value: serde_json::Value,
}

#[derive(Deserialize)]
struct BulkSettingsUpdateRequest {
    settings: HashMap<String, serde_json::Value>,
}
```

## Implementation Steps

### Phase 1: Core Settings Framework
1. **Settings Registry**: Create a static registry of supported settings with metadata
2. **Validation Rules**: Define validation logic for each setting type
3. **Environment Override Detection**: Check for env var precedence
4. **Default Values**: Define fallback defaults for each setting

### Phase 2: CRUD Endpoints
1. **GET /settings**: List all settings with current values, defaults, validation rules
2. **GET /settings/{key}**: Get specific setting with metadata
3. **PUT /settings/{key}**: Update single setting with validation
4. **DELETE /settings/{key}**: Reset to default value
5. **PUT /settings**: Bulk update multiple settings

### Phase 3: Setting-Specific Logic
1. **GOOSE_MODE**: Validate against enum (auto/approve/smart_approve/chat)
2. **GOOSE_TEMPERATURE**: Validate range 0.0-2.0, number format
3. **GOOSE_MAX_TURNS**: Validate positive integer, minimum 1
4. **GOOSE_CLI_MIN_PRIORITY**: Validate range 0.0-1.0
5. **GOOSE_RECIPE_GITHUB_REPO**: Validate "owner/repo" format
6. **GOOSE_AUTO_COMPACT_THRESHOLD**: Validate positive integer

## Key Considerations

### Environment Variable Precedence
- **Read-only when env set**: If env var exists, API should return read-only status
- **Warning in response**: Include env_override field when applicable
- **No modification**: Prevent updates when env var takes precedence

### Validation Strategy
- **Type checking**: Ensure string/number/boolean types match
- **Range validation**: Min/max values for numbers
- **Enum validation**: Allowed string values for modes
- **Format validation**: Regex patterns for repo names

### Server Restart Requirements
- **Model changes**: Require server restart to take effect
- **Mode changes**: May require agent reconfiguration
- **Note in API response**: Include restart_required field

### Default Values Discovery
```rust
// From configure.rs patterns:
let current_max_turns: u32 = config.get_param("GOOSE_MAX_TURNS").unwrap_or(1000);
let current_mode: String = config.get_param("GOOSE_MODE").unwrap_or("auto".to_string());
```

## Integration with Existing Code

### Configuration Manager Usage
- Follow `configure.rs` patterns for config access
- Use existing validation logic from dialog functions
- Maintain compatibility with CLI configuration

### Error Handling
- **ConfigError types**: Handle NotFound, DeserializeError, FileError
- **HTTP status codes**: 400 for validation errors, 500 for config errors
- **Detailed error messages**: Include validation failure reasons

### API Response Format
- **Setting metadata**: Include type, validation, defaults
- **Environment status**: Show if overridden by env vars
- **Restart hints**: Indicate when changes require restart

This approach provides a clean REST API for runtime configuration management while maintaining full compatibility with the existing CLI configuration system.