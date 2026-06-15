# Resilient MCP Server - Deployment Decision

## Decision: Railway.app (Starter) → VPS (Long-term)

### Rationale

**Railway** (immediate, 1-2 weeks):
- ✅ Auto-deploy on git push (zero config)
- ✅ Free tier: $5/mo (easy to test)
- ✅ Zero infrastructure management
- ✅ Integrated monitoring + logging
- ❌ Vendor lock-in (but easy to migrate)

**VPS** (after validation, weeks 3+):
- ✅ Full control, cheaper long-term
- ✅ Can self-host if needed
- ✅ No vendor dependency
- ✅ Multiple services on one box
- ❌ Manual deployment, patching

### Action Plan

#### **Phase 1: Railway Setup (Week 1)**

1. **Create Dockerfile**
   ```dockerfile
   FROM rust:1.75-alpine AS builder
   WORKDIR /app
   COPY . .
   RUN cargo build --release --manifest-path resilient/Cargo.toml
   
   FROM alpine:latest
   RUN apk add --no-cache curl ca-certificates libz3
   COPY --from=builder /app/resilient/target/release/rz /usr/local/bin/
   EXPOSE 8080
   CMD ["rz", "--mcp-server", "--http-port", "8080"]
   ```

2. **Deploy**
   ```bash
   railway login
   railway link EricSpencer00/Resilient
   railway variable set Z3_BINARY=/usr/bin/z3
   railway up
   ```

3. **Monitor**
   - Dashboard: railway.app/projects/resilient
   - Logs: real-time tail in UI
   - Health: curl https://resilient-mcp.railway.app/health

#### **Phase 2: Validation (Week 2)**
- [ ] Claude Code integration test
- [ ] 24-hour uptime verification
- [ ] Cost confirmation
- [ ] API performance baseline

#### **Phase 3: VPS Migration (Weeks 3+)**
- [ ] Provision Hetzner/DigitalOcean box ($5-10/mo)
- [ ] Deploy via systemd + auto-restart
- [ ] Migrate DNS to VPS
- [ ] Archive Railway

## MCP Tools to Expose

### Tier 1 (MVP - Week 1)
```json
{
  "tools": [
    {
      "name": "rz_compile",
      "description": "Compile Resilient code and return diagnostics",
      "input_schema": {
        "type": "object",
        "properties": {
          "code": { "type": "string" }
        }
      }
    },
    {
      "name": "rz_format",
      "description": "Format Resilient code",
      "input_schema": {
        "type": "object",
        "properties": {
          "code": { "type": "string" }
        }
      }
    }
  ]
}
```

### Tier 2 (If time - Week 2)
```json
{
  "name": "rz_verify",
  "description": "Verify contracts with Z3",
  "input_schema": {
    "type": "object",
    "properties": {
      "code": { "type": "string" },
      "verify_contracts": { "type": "boolean", "default": true }
    }
  }
}
```

## HTTP API Specification

```
POST /mcp/call
Content-Type: application/json

{
  "tool": "rz_compile",
  "input": {
    "code": "const X = 5; println(X);"
  }
}

←

{
  "status": "ok",
  "stderr": "",
  "stdout": "",
  "diagnostics": []
}
```

## Costs

| Service | Monthly | Notes |
|---------|---------|-------|
| **Railway** | $5-20 | Free tier includes 500 compute hours; easy to scale |
| **Hetzner VPS** | $5.99 | 2 vCPU, 4GB RAM; plenty for single MCP server |
| **Domain** | $12/yr | resilient-mcp.dev or similar |
| **Monitoring** | Free (built-in) | Railway provides; use Healthchecks.io if self-hosted |

**Total**: $15-40/month (very reasonable for production service)

## Security

- [ ] API rate limiting: 100 req/min per IP
- [ ] Input validation: max 10MB code per request
- [ ] Timeout: 10s per compile
- [ ] No secrets in logs
- [ ] TLS only (https)
- [ ] CORS: Claude Code origins only

## Success Metrics

- [ ] Endpoint responds in <2s (p95)
- [ ] 99.5% uptime SLA
- [ ] Zero authentication errors from Claude
- [ ] <$20/mo cost
- [ ] Rollback-able in <5 min

## Next Steps

1. **Today**: Create Dockerfile + test locally
2. **Tomorrow**: Push to Railway
3. **Week 1**: Validate with Claude Code
4. **Week 2**: Plan VPS migration
