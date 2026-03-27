# Aide — commandes supportées (clos-monitor / server-monitor)

Ce document liste **toutes** les commandes du binaire et du bot Telegram. Le message `/help` dans Telegram est généré à partir de la configuration (blocs Kylit et IP admin affichés seulement s’ils sont activés).

## Normalisation (Telegram)

- Suffixe de bot en groupe : `/help@MonBot` → `/help`
- Séparateurs équivalents : tiret, tiret long, deux-points → underscore (ex. `/status-prod` → `/status_prod`)

---

## Bot Telegram — sans `TELEGRAM_ALLOWED_CHAT_IDS`

Ces commandes répondent **sans** être dans la liste des chats autorisés (pour obtenir un ID et la liste des commandes).

| Commande | Description |
|----------|-------------|
| `/start` | Message d’accueil + rappel d’ajouter le chat ID à la config |
| `/myid` | Affiche votre `chat_id` à copier dans `TELEGRAM_ALLOWED_CHAT_IDS` |
| `/help` | Aide HTML (équivalent de ce document, selon la config active) |

---

## Bot Telegram — chats autorisés uniquement

Toutes les commandes ci‑dessous exigent que l’expéditeur soit dans `TELEGRAM_ALLOWED_CHAT_IDS` (ou `TELEGRAM_CHAT_ID` selon la config).

### Statut et diagnostic

| Commande | Description |
|----------|-------------|
| `/status` | État complet (prod, staging, serveur) |
| `/status_prod` | Production uniquement |
| `/status_staging` | Staging uniquement |
| `/status_server` | Vitales serveur uniquement |
| `/self` | Auto‑diagnostic (chaîne webhook : env, PID, port local, Telegram `getWebhookInfo`, nginx) |

### Statistiques serveur

| Commande | Description |
|----------|-------------|
| `/space_left` | Espace disque (/ , /var, /home) |
| `/uptime_stats` | Uptime + charge |
| `/memory` | Mémoire |
| `/certs` | Expiration des certificats SSL surveillés |
| `/docker` | Liste des conteneurs prod / staging (noms via `MONITOR_*_CONTAINERS`) |

### Production

| Commande | Description |
|----------|-------------|
| `/prod_backup` | Sauvegarde PostgreSQL (conteneur `MONITOR_PROD_PG_CONTAINER`) |
| `/prod_restart` | Redémarrage compose (`MONITOR_PROD_COMPOSE_RESTART_SH` après `cd` deploy prod) |

### Staging

| Commande | Description |
|----------|-------------|
| `/staging_backup` | Sauvegarde PostgreSQL (`MONITOR_STAGING_PG_CONTAINER`) |
| `/staging_restart` | Redémarrage compose (`MONITOR_STAGING_COMPOSE_RESTART_SH`) |

### Aide (répétition)

| Commande | Description |
|----------|-------------|
| `/help` | Même contenu que pour les utilisateurs publics, avec éventuels blocs Kylit / IP |

### Kylit (optionnel)

Activé seulement si `MONITOR_KYLIT_WEBHOOK=1` (voir `env.kylit.example`).

| Commande | Description |
|----------|-------------|
| `/kylit_backup_db` | Dump Postgres vers `KYLIT_BACKUP_ROOT` |
| `/kylit_backup_minio` | Exécute le script mirror MinIO du déploiement |
| `/kylit_backup_all` | Script `kylit-prod-backup.sh` |
| `/kylit_docker` | État des conteneurs (`MONITOR_KYLIT_CONTAINERS`) |

Si Kylit est désactivé, ces commandes renvoient un message d’information (pas une erreur fatale).

### IP admin — UFW ou CrowdSec (optionnel)

Activé si `MONITOR_IP_BACKEND=ufw` ou `crowdsec` **et** `MONITOR_IP_ADMIN_SECRET` est défini. Le mot de passe doit avoir **exactement** la même longueur que le secret (comparaison constante en temps).

| Commande | Syntaxe | Description |
|----------|---------|-------------|
| `/ip_list` | `/ip_list <mot_de_passe>` | Liste (`ufw status numbered` ou `cscli decisions list`) |
| `/ip_ban` | `/ip_ban <ipv4> <mot_de_passe>` | Bannir une IP |
| `/ip_unban` | `/ip_unban <ipv4> <mot_de_passe>` | Lever le ban |

**Sécurité :** Telegram n’est pas un canal confidentiel ; ne pas réutiliser un mot de passe sensible ailleurs.

---

## Ligne de commande (CLI)

| Commande | Description |
|----------|-------------|
| `clos-monitor check` | Vérification unique (code de sortie 1 si échec) |
| `clos-monitor check --dev` | Mode dev (URLs locales compose, voir env) |
| `clos-monitor check --scope prod\|staging\|both` | Surcharge du périmètre |
| `clos-monitor run` | Boucle daemon + alertes Telegram |
| `clos-monitor run --dev` / `--scope …` | Idem avec options |
| `clos-monitor diagnose` | Diagnostic chaîne webhook (stdout, même logique que `/self`) |

---

## Documentation associée

- [SETUP.md](SETUP.md) — variables d’environnement et déploiement
- [TELEGRAM_WEBHOOK.md](TELEGRAM_WEBHOOK.md) — webhook, secret, nginx ou Traefik, vérification au démarrage
- [DEPLOY_CHECKLIST.md](DEPLOY_CHECKLIST.md) — checks après déploiement
