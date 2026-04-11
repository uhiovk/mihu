# Mihu: A lightweight subscription manager for Clash & Mihomo

Configurations, subscriptions and overrides are stored at `<config-dir>/mihu`.

The program has a global override profile that applies to all subscriptions, and subscription-specific overrides. You can edit them by running `mihu edit`.

## Usage

Please run `mihu init [-c PATH-TO-MIHOMO-CONFIG] [-u MIHOMO-EXTCTL-ENDPOINT]` before using for the first time.

These two settings can be only modified later by manually editing the config file at `<config-dir>/mihu/config.yaml`.

### Update current subscription
```
mihu
```

Same as `mihu update`.

### Add a new subscription
```
mihu sub <NAME> <URL> [-s] [-d]
```

Can also edit an existing one. Add `-s` to switch to it. Add `-d` to set it as default.

### Delete a aubscription
```
mihu remove <NAME> [-o]
```

Alias: `rm`. Add `-o` to remove custom override file.

If this is the current or default subscription, it will fallback to a random one.

### Switch subscription
```
mihu switch [NAME] [-u] [-d]
```

Alias: `sw`. Emit name to use the default one. Add `-u` to update it. Add `-d` to set it as default.

### Update subscriptions
```
mihu update [NAME]... [-a]
```

Alias: `up`. Emit name to update the current one. Add `-a` to update all subscriptions.

---

Check help messages for other subcommands.

## Override profile example
```yaml
# non-string keys are ignored

# directly rewrite simple items
mixed-port: 7890
external-controller: 127.0.0.1:9090

# only effect on the most inner item(s) in the map
profile:
  store-selected: true

# rewrite the whole list
authentication:
  - sudo:gimmeasandwich
skip-auth-prefixes:
  - 127.0.0.0/8
  - 10.0.0.0/8
  - 172.16.0.0/12

# rewrite the whole map
dns!:
  enable: true
  ipv6: true
  prefer-h3: true
  respect-rules: false
  enhanced-mode: normal
  default-nameserver:
    - 223.5.5.5
  nameserver:
    - https://dns.alidns.com/dns-query

# add new items at the start of a list
+rules:
  - DOMAIN-SUFFIX,bilibili.com,DIRECT
  - DOMAIN-SUFFIX,bilivideo.com,DIRECT
  - DOMAIN-SUFFIX,hdslb.com,DIRECT

# add new items at the end of a list
rules+:
  - MATCH,Fallback

# these patterns can be nested in a map
dns:
  default-nameserver:
    - 1.1.1.1
  nameserver+:
    - https://dns.cloudflare.com/dns-query

# wrap the key inside "< >" to avoid ambiguity
<C++>:
  - the worst programming language
```
