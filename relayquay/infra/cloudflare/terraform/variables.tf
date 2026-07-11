variable "cloudflare_account_id" {
  description = "Cloudflare account ID."
  type        = string
  sensitive   = true
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID for the public hostname."
  type        = string
  sensitive   = true
}

variable "tunnel_id" {
  description = "Existing Cloudflare Tunnel ID."
  type        = string
  sensitive   = true
}

variable "public_hostname" {
  description = "Public hostname routed to RelayQuay. For cover deployments, do not attach Cloudflare Access to this hostname."
  type        = string
}

variable "origin_service" {
  description = "Origin service configured in the tunnel ingress."
  type        = string
  default     = "http://127.0.0.1:8788"
}

variable "access_application_domain" {
  description = "Optional Cloudflare Access application domain. Leave empty for cover-only public hostnames; use a separate protected hostname when Access is required."
  type        = string
  default     = ""
}

variable "create_access_dns_record" {
  description = "Create a proxied DNS record for access_application_domain."
  type        = bool
  default     = true
}

variable "access_application_name" {
  description = "Cloudflare Access application name."
  type        = string
  default     = "RelayQuay"
}

variable "access_session_duration" {
  description = "Cloudflare Access session duration."
  type        = string
  default     = "24h"
}

variable "allowed_emails" {
  description = "Email identities allowed by the Access policy when access_application_domain is set."
  type        = list(string)
  default     = []
}
