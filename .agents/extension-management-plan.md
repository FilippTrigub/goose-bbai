# Extension Management Web UI Integration Plan

## 🎯 Objective
Integrate Goose API extension management endpoints into the K8s Manager web app at project level.

## 📡 Available API Endpoints
```rust
// Extension Management Routes
GET    /api/v1/extensions               // List all extensions
POST   /api/v1/extensions               // Create new extension  
PUT    /api/v1/extensions/{name}/toggle // Enable/disable extension
DELETE /api/v1/extensions/{name}        // Remove extension (if disabled)
```

## 🏗️ Implementation Plan

### 1. Data Models (Python)
```python
# Add to models.py
class Extension(BaseModel):
    name: str
    display_name: Optional[str]
    extension_type: str  # builtin, stdio, sse, streamable_http, frontend, inline_python
    enabled: bool
    timeout: Optional[int]
    description: Optional[str]
    # Type-specific fields...

class ExtensionCreate(BaseModel):
    name: str
    extension_type: str
    # ... other fields based on type
```

### 2. Backend Proxy Routes (Python)
```python
# Add to project_routes.py
GET    /users/{user_id}/projects/{project_id}/extensions
POST   /users/{user_id}/projects/{project_id}/extensions  
PUT    /users/{user_id}/projects/{project_id}/extensions/{name}/toggle
DELETE /users/{user_id}/projects/{project_id}/extensions/{name}
```

### 3. Frontend UI Components
- **Extensions Tab** in project cards/detail view
- **Extension List** with enable/disable toggles
- **Create Extension Modal** with type-specific form fields
- **Extension Status Indicators** (enabled/disabled badges)

### 4. UI Integration Points
- Add "🧩 Extensions" button to project action grid (becomes 3×2 grid)
- Show extension count in project metadata
- Extension management modal/panel when project is active

### 5. Extension Types Support
- **builtin**: Simple toggle interface
- **stdio**: Form for cmd, args, envs
- **sse/http**: URI and headers configuration  
- **python**: Code editor for inline Python
- **frontend**: Instructions/tools configuration

## ⚡ Quick Implementation Steps
1. Add Extension models to `models.py`
2. Add proxy routes to `project_routes.py` 
3. Add "🧩 Extensions" button to project cards
4. Create extension management modal/panel
5. Implement CRUD operations with real-time updates
6. Add extension status to project metadata display

## 🎨 UI Design
- **Extension Cards** similar to project cards but smaller
- **Toggle Switches** for enable/disable
- **Type Badges** (builtin, stdio, python, etc.)
- **Status Indicators** (✅ enabled, ⏸️ disabled)
- **Quick Actions** (edit, delete, toggle)

## 🔐 Security Notes
- Extensions are project-scoped (isolated per project environment)
- Only active projects can modify extensions
- Extension configs stored in Goose API (not K8s Manager MongoDB)
- Validate extension types and required fields

**Total Effort**: ~4-6 hours implementation for complete extension management UI integration.