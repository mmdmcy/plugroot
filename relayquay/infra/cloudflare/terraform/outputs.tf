output "public_hostname" {
  value = var.public_hostname
}

output "dns_record_id" {
  value = cloudflare_record.relayquay.id
}

output "access_dns_record_id" {
  value = try(cloudflare_record.relayquay_access[0].id, null)
}

output "access_application_id" {
  value = try(cloudflare_access_application.relayquay[0].id, null)
}

output "access_application_domain" {
  value = var.access_application_domain != "" ? var.access_application_domain : null
}
