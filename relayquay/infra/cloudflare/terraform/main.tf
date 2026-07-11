resource "cloudflare_record" "relayquay" {
  zone_id = var.cloudflare_zone_id
  name    = var.public_hostname
  type    = "CNAME"
  content = "${var.tunnel_id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
}

locals {
  access_enabled = var.access_application_domain != ""
}

resource "cloudflare_record" "relayquay_access" {
  count = local.access_enabled && var.create_access_dns_record ? 1 : 0

  zone_id = var.cloudflare_zone_id
  name    = var.access_application_domain
  type    = "CNAME"
  content = "${var.tunnel_id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
}

resource "cloudflare_tunnel_config" "relayquay" {
  account_id = var.cloudflare_account_id
  tunnel_id  = var.tunnel_id

  config {
    ingress_rule {
      hostname = var.public_hostname
      service  = var.origin_service
    }

    dynamic "ingress_rule" {
      for_each = local.access_enabled ? [var.access_application_domain] : []

      content {
        hostname = ingress_rule.value
        service  = var.origin_service
      }
    }

    ingress_rule {
      service = "http_status:404"
    }
  }
}

resource "cloudflare_access_application" "relayquay" {
  count = local.access_enabled ? 1 : 0

  account_id           = var.cloudflare_account_id
  name                 = var.access_application_name
  domain               = var.access_application_domain
  type                 = "self_hosted"
  session_duration     = var.access_session_duration
  app_launcher_visible = false
}

resource "cloudflare_access_policy" "allow_users" {
  count = local.access_enabled ? 1 : 0

  account_id     = var.cloudflare_account_id
  application_id = cloudflare_access_application.relayquay[0].id
  name           = "Allow RelayQuay users"
  precedence     = 1
  decision       = "allow"

  include {
    email = var.allowed_emails
  }
}
