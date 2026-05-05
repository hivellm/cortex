# HiveHub.Cloud: Public Surface (APIs & Routes)

**Status**: REST API fully specified and implemented; dashboard frontend not yet started

---

## REST API Endpoints

**Base URL**: `http://localhost:3000` (dev) | `https://api.hivehub.cloud` (prod)  
**Documentation**: OpenAPI 3.0 at `docs/api/openapi.yaml` | Swagger UI at `/api/docs`  
**Format**: JSON request/response  
**Authentication**: JWT Bearer token in `Authorization: Bearer <token>` header (except login/register/health)

---

## Authentication Endpoints

### POST `/api/auth/register`

Register new user with email and password.

**Request**:
```json
{
  "email": "user@example.com",
  "password": "SecurePassword123!",
  "name": "John Doe" (optional)
}
```

**Response** (201 Created):
```json
{
  "user": {
    "id": "uuid",
    "email": "user@example.com",
    "name": "John Doe",
    "created_at": "2025-12-04T10:00:00Z"
  },
  "tokens": {
    "access_token": "eyJ...",
    "refresh_token": "eyJ...",
    "expires_in": 3600
  }
}
```

### POST `/api/auth/login`

Login with email and password.

**Request**:
```json
{
  "email": "user@example.com",
  "password": "SecurePassword123!"
}
```

**Response** (200 OK): Same as register

### POST `/api/auth/refresh`

Refresh JWT access token using refresh token.

**Request**:
```json
{
  "refresh_token": "eyJ..."
}
```

**Response**: New access and refresh tokens

### GET `/api/auth/oauth/google`

Initiate Google OAuth flow. Redirects to Google consent screen.

### GET `/api/auth/oauth/google/callback`

OAuth callback from Google. Creates/links user and returns tokens.

### GET `/api/auth/oauth/github`

Initiate GitHub OAuth flow.

### GET `/api/auth/oauth/github/callback`

OAuth callback from GitHub.

### POST `/api/auth/logout`

Logout (invalidate refresh tokens). **Requires auth.**

---

## User Endpoints

### GET `/api/users/me`

Get current authenticated user. **Requires auth.**

**Response**:
```json
{
  "id": "uuid",
  "email": "user@example.com",
  "name": "John Doe",
  "subscription": {
    "plan": "free",
    "status": "active",
    "renews_at": "2026-01-04"
  },
  "created_at": "2025-12-04T10:00:00Z"
}
```

### PUT `/api/users/me`

Update user profile. **Requires auth.**

**Request**:
```json
{
  "name": "Jane Doe",
  "email": "newemail@example.com"
}
```

---

## Subscription Endpoints

### GET `/api/subscriptions/plans`

Get all available subscription plans. **Public (no auth required).**

**Response**:
```json
{
  "plans": [
    {
      "id": "free",
      "name": "Free",
      "price_cents": 0,
      "billing_cycle": "monthly",
      "limits": {
        "documents": 1000,
        "storage_bytes": 52428800,
        "collections": 1,
        "api_requests_per_month": 200
      }
    },
    {
      "id": "pro",
      "name": "Pro",
      "price_cents": 2000,
      ...
    }
  ]
}
```

### GET `/api/subscriptions/current`

Get current user's subscription. **Requires auth.**

**Response**:
```json
{
  "id": "uuid",
  "user_id": "uuid",
  "plan": "free",
  "status": "active",
  "started_at": "2025-12-04T00:00:00Z",
  "renews_at": "2026-01-04T00:00:00Z",
  "usage": {
    "documents": 150,
    "storage_bytes": 10485760,
    "api_requests_month": 45
  }
}
```

### POST `/api/subscriptions/upgrade`

Upgrade to a higher plan. Creates Stripe/PayPal checkout session. **Requires auth.**

**Request**:
```json
{
  "plan_id": "pro",
  "payment_provider": "stripe"
}
```

**Response**:
```json
{
  "checkout_url": "https://checkout.stripe.com/...",
  "session_id": "cs_..."
}
```

### GET `/api/subscriptions/usage`

Get detailed usage metrics. **Requires auth.**

**Response**:
```json
{
  "document_count": 150,
  "storage_bytes": 10485760,
  "api_requests_month": 45,
  "collections": 1,
  "limit_warnings": [
    {
      "metric": "storage",
      "current": 10485760,
      "limit": 52428800,
      "percentage_used": 20
    }
  ]
}
```

---

## Project Endpoints

### GET `/api/projects`

List user's projects. **Requires auth.**

### POST `/api/projects`

Create new project. **Requires auth.**

**Request**:
```json
{
  "name": "My Project",
  "description": "Project description"
}
```

### GET `/api/projects/:projectId`

Get project details including collections and graphs. **Requires auth.**

### PUT `/api/projects/:projectId`

Update project metadata. **Requires auth.**

### DELETE `/api/projects/:projectId`

Delete project and cascade collections/graphs. **Requires auth.**

---

## File Upload Endpoints

### POST `/api/files/upload`

Upload file for processing. **Requires auth; multipart/form-data.**

**Request** (multipart):
```
POST /api/files/upload
Authorization: Bearer <token>
Content-Type: multipart/form-data

--boundary
Content-Disposition: form-data; name="file"; filename="document.pdf"
Content-Type: application/pdf

<binary file data>
--boundary
Content-Disposition: form-data; name="project_id"

<project-uuid>
--boundary--
```

**Response** (202 Accepted):
```json
{
  "file_id": "uuid",
  "status": "processing",
  "message": "File queued for Transmutation and indexing"
}
```

### GET `/api/files/:fileId`

Get file metadata and processing status. **Requires auth.**

**Response**:
```json
{
  "id": "uuid",
  "name": "document.pdf",
  "content_type": "application/pdf",
  "status": "indexed",
  "chunks_count": 42,
  "vectors_stored": 42,
  "uploaded_at": "2025-12-04T10:00:00Z"
}
```

### GET `/api/projects/:projectId/files`

List files in project. **Requires auth.**

### DELETE `/api/files/:fileId`

Delete file and remove vectors from Vectorizer. **Requires auth.**

---

## Payment Webhooks

### POST `/api/webhooks/stripe`

Stripe webhook endpoint (payment status updates). **Public (signature verified).**

**Events handled**:
- `payment_intent.succeeded` – Update subscription status
- `invoice.payment_failed` – Mark invoice as failed; notify user

### POST `/api/webhooks/paypal`

PayPal webhook endpoint. **Public (signature verified).**

---

## Health & Metrics

### GET `/health`

Health check (database + service endpoints). **Public (no auth).**

**Response**:
```json
{
  "status": "ok",
  "database": "connected",
  "services": {
    "vectorizer": "ok",
    "nexus": "ok",
    "synap": "ok"
  }
}
```

### GET `/api/metrics`

System metrics (request count, error rates, latencies). **Admin only.**

---

## MCP Server Endpoints (Planned)

### POST `/api/mcp-servers`

Create authenticated MCP server endpoint for a Hive service. **Requires auth.**

**Request**:
```json
{
  "service": "vectorizer",
  "project_id": "uuid",
  "name": "My Vectorizer Server"
}
```

**Response**:
```json
{
  "id": "uuid",
  "endpoint": "https://api.hivehub.cloud/mcp/servers/<server-id>",
  "api_key": "mcp_sk_...",
  "service": "vectorizer"
}
```

---

## Internal Service API Endpoints

**For Hive services only** (service API key auth).

See [05 - integrations.md](./05%20-%20integrations.md) and [SERVICE_API.md](../../HivehubCloud/docs/specs/SERVICE_API.md) for full details.

**Examples**:
- `GET /api/internal/vectorizer/user/{userId}/collections`
- `GET /api/internal/nexus/user/{userId}/databases`
- `POST /api/internal/{service}/usage/update`

---

## Dashboard Routes (Planned, Not Yet Started)

| Route | Purpose |
|-------|---------|
| `/` | Dashboard home |
| `/auth/login` | Login page |
| `/auth/register` | Registration page |
| `/auth/oauth/callback` | OAuth callback handler |
| `/projects` | Projects list |
| `/projects/:projectId` | Project detail + files + collections |
| `/subscriptions` | Subscription management + upgrade |
| `/account` | Account settings + profile |
| `/admin` | Admin dashboard (analytics, users) |

**Status**: React + TypeScript + Vite; not yet started.

---

## API Versioning

Currently at **v1** (implicit in `/api/` prefix).

Future: May introduce `/api/v2` if breaking changes needed.

---

## Rate Limiting (Planned)

- 100 requests/second per user (future)
- 10 requests/second per service API key (future)
- 429 Too Many Requests response on limit exceeded
