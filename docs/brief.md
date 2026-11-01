# Brief — Nuage

**Client de stockage cloud pour Linux : un démon Rust qui pilote rclone, une application GNOME (Flatpak) pour le bureau et une CLI pour les serveurs headless.**

## En bref

Sous Linux, monter un Google Drive ou un OneDrive est pénible : rclone sait tout faire mais n'a pas d'interface simple. Nuage :

- monte chaque compte cloud dans un dossier choisi par l'utilisateur, visible dans Nautilus ;
- sur le bureau, affiche l'état de la synchronisation et l'espace utilisé dans une icône de la barre système ;
- offre **un seul navigateur de fichiers, identique pour tous les fournisseurs** ;
- se met en route en **quelques écrans guidés** (4 pour un premier compte Google, 3 ensuite), puis vit dans la barre système ;
- **s'installe et s'utilise entièrement en ligne de commande** sur un serveur headless accessible en SSH, y compris par un agent comme Claude Code.

v1 : Google Drive, **public dès la v1**. L'architecture est multi-fournisseurs dès le départ.

## Consignes pour Claude Code

1. **Ordre de travail** : suivre les [jalons](#jalons) dans l'ordre. Le jalon 1 (preuve que FUSE fonctionne depuis le Flatpak) passe avant tout le reste ; s'il échoue, s'arrêter et en parler.
2. **Git** : le dépôt est `git@github.com:felixilgatto/nuage.git` (https://github.com/felixilgatto/nuage), créé **manuellement** par Felix. Ne pas créer d'autre dépôt distant. Commits et push uniquement sur demande.
3. **Langue** : ce brief est en français, mais **tout ce qui est produit est en anglais** : code, identifiants (y compris les variantes d'enum), commentaires, messages de commit, README, `docs/`, chaînes de l'interface et de la CLI. Les chaînes de l'UI passent par gettext pour pouvoir être traduites plus tard.
4. **Modularité** : l'architecture modulaire (voir [Architecture](#architecture)) est en place dès le premier commit, même avec un seul fournisseur.
5. **Licence MIT** : fichier `LICENSE` à la racine, `license = "MIT"` dans chaque `Cargo.toml`, `<project_license>MIT</project_license>` dans le `metainfo.xml`. `cargo-deny` n'accepte que des licences compatibles MIT (MIT, Apache-2.0, BSD, ISC, Zlib ; MPL-2.0 au cas par cas ; GPL/AGPL refusées). rclone est sous MIT, on peut donc l'embarquer.
6. **Noms** : application **Nuage** ; binaires `nuaged` (démon), `nuage` (CLI), `nuage-gtk` (application de bureau) ; crates `nuage-*` ; identifiant Flatpak **`io.github.felixilgatto.Nuage`**.
7. **Points à vérifier** : les hypothèses techniques non confirmées sont regroupées dans [À vérifier](#à-vérifier). Les vérifier au moment indiqué plutôt que de les supposer vraies.

## Principes

Ces règles tranchent les cas non prévus par le brief.

- **On ne réécrit pas de moteur de sync.** rclone fait le montage, le cache, l'OAuth et les transferts. Nuage est une interface au-dessus de son API de contrôle (rc). Seule exception : le système de fichiers superposé `nuage-fs` (exclusions exactes en v1, épinglage hors ligne en v1.1).
- **Un moteur, plusieurs interfaces.** Le moteur tourne dans un démon (`nuaged`). L'application GTK, l'icône de barre système et la CLI en sont des clients. **Tout ce que fait l'interface graphique est faisable en CLI**, à l'exception du navigateur graphique.
- **Headless de première classe.** Sur un serveur sans écran, Nuage s'installe, se configure et s'utilise uniquement en ligne de commande, sans aucune étape interactive obligatoire.
- **Public dès la v1.** Messages d'erreur compréhensibles par un non-technicien, avec l'action à faire ; README d'installation pas à pas.
- **Fichiers uniquement.** Ni calendrier, ni contacts, ni mail, ni photos. Les scopes OAuth se limitent aux fichiers.
- **Interface minimaliste.** Le moins d'écrans, de choix et de réglages visibles possible. Tout ce qui a un bon défaut n'est pas demandé et reste dans les réglages. Pas d'écran d'accueil, de tutoriel ni de récapitulatif.
- **Indépendance vis-à-vis du fournisseur.** Tout ce qui dépend d'un fournisseur est dans son module. Les interfaces s'adaptent aux **capacités** du fournisseur, jamais à son nom.
- **GNOME natif** sur le bureau : GTK4 + libadwaita, GNOME HIG, guidelines de qualité Flathub.
- **Sandbox aussi fermé que possible.** Les permissions Flatpak sont minimales et chacune est justifiée dans le README.

## Périmètre par version

| Fonctionnalité | v1 | v1.1 | v2 | Jamais |
|---|---|---|---|---|
| Google Drive (« Mon Drive ») | ✔ | | | |
| Drive partagés, « Partagés avec moi » | | | ✔ (ou plus tard) | |
| Fournisseur `local` (tests, CI) | ✔ | | | |
| OneDrive, autres fournisseurs | possible tôt, pour valider l'abstraction | | ✔ | |
| Montage par compte, dossier au choix | ✔ | | | |
| Démon `nuaged` + CLI `nuage` | ✔ | | | |
| Installation headless (binaire natif, service systemd) | ✔ | | | |
| Application GTK + icône de barre système (Flatpak) | ✔ | | | |
| Quota par compte | ✔ | | | |
| Limite du cache local, par compte | ✔ | | | |
| Pause / reprise des uploads | ✔ | | | |
| Navigateur de fichiers graphique unifié | ✔ | | | |
| Opérations sur les fichiers en CLI (`ls`, `cp`, `mv`…) | ✔ | | | |
| Mise en route guidée (client OAuth perso compris), démarrage auto | ✔ | | | |
| Système de fichiers superposé `nuage-fs` | ✔ relais + exclusions | ✔ + épinglage | | |
| Exclusions (chemins et motifs) | ✔ via `nuage-fs` + filtres rclone | | | |
| Mode hors ligne (épinglage) | | ✔ | | |
| CI GitHub Actions, releases (bundle Flatpak + binaires natifs) | ✔ | | | |
| Publication sur Flathub | | | ✔ | |
| Interface texte (TUI) ou web | | | | ✘ (la CLI suffit) |
| Badges d'état dans Nautilus | | | hors Flatpak, éventuellement | impossible depuis un Flatpak |
| Miroir hors ligne de tout le compte | | | | ✘ (seuls les éléments épinglés) |
| Résolution de conflits pour les fichiers non épinglés | | | | ✘ (on s'en remet au VFS de rclone) |
| Édition des Google Docs hors du navigateur web | | | | ✘ (raccourcis web, voir [Google Drive](#comptes-et-authentification)) |
| Calendrier, contacts, mail, photos, documents collaboratifs | | | | ✘ |

## Existant (évalué le 2026-10-30)

**RClone Manager** (`io.github.zarestia_dev.rclone-manager`, Flathub, Tauri/Rust, maintenu) couvre une partie du besoin côté bureau : montage, icône tray, plusieurs fournisseurs, navigateur de fichiers. Mais :

- **son Flatpak ne monte pas tel quel** : le manifeste ne déclare ni `--talk-name=org.freedesktop.Flatpak` ni `--filesystem=home`, et l'utilisateur doit faire un `flatpak override` manuel (issues #45, #52, #113 ; le mainteneur indique que Flathub refuse ces permissions) ;
- **rclone n'est pas embarqué** : l'application télécharge la dernière version au premier lancement, sans version épinglée ;
- il affiche le quota (`operations/about`) mais **pas d'état « à jour / en sync »** (il ne lit pas `vfs/stats`) ;
- il n'est pas GNOME natif (Tauri/WebKit).

Nuage se justifie donc par une UX GNOME native, simple et ciblée, un montage qui marche sans réglage manuel, un état de sync clair, et un mode headless scriptable. Son wrapper `fusermount3` reste une référence utile.

## Deux modes de distribution

| | Bureau | Headless |
|---|---|---|
| Cible | Ubuntu / GNOME avec session graphique | serveur Linux sans écran, administré en SSH |
| Paquet | Flatpak (bundle `.flatpak`, Flathub en v2) | binaire natif : tarball x86_64 et `.deb` |
| Contenu | `nuaged` + `nuage-gtk` + `nuage` + rclone + wrapper `fusermount3` | `nuaged` + `nuage` + rclone (même version épinglée) + unit systemd ; **aucune dépendance GTK** |
| Lancement du démon | par `nuage-gtk` (démarrage auto via le portail Background) | service systemd utilisateur `nuaged.service` ; `loginctl enable-linger` pour tourner sans session ouverte |
| FUSE | `fusermount3` de l'hôte, via le wrapper (voir [point dur](#le-point-dur--fuse-dans-le-sandbox-flatpak)) | `fusermount3` du système, directement |
| Secrets | portail Secret | Secret Service si disponible, sinon fichier en `0600` (voir [Sécurité](#sécurité-et-stockage)) |
| Authentification | navigateur ouvert automatiquement | URL + tunnel SSH, ou token généré ailleurs (voir [Authentification](#comptes-et-authentification)) |
| Pré-requis hôte | `flatpak`, `fuse3` | `fuse3`, systemd |

## Le point dur : FUSE dans le sandbox Flatpak

Ne concerne que la distribution Flatpak. Le sandbox n'a ni `fusermount3` (binaire setuid) ni le droit de monter. Le contournement **éprouvé** est celui de GNOME Builder, repris par RClone Manager :

1. Le Flatpak installe un script `/app/bin/fusermount3` qui remplace le vrai.
2. Quand rclone (dans le sandbox) monte, il appelle ce script, qui relaie l'appel au **`fusermount3` de l'hôte** via `flatpak-spawn --host`, en transmettant les descripteurs de fichiers (`_FUSE_COMMFD`, `/dev/fd/N`) avec `--forward-fd`.
3. Le montage est fait par l'hôte, donc **visible dans Nautilus**. rclone reste dans le sandbox et lit le descripteur `/dev/fuse` qu'on lui renvoie.

**Complication sur Ubuntu 26.04 (constatée au jalon 1).** Le profil AppArmor de `fusermount3` (`/etc/apparmor.d/fusermount3`, fuse3 3.18) n'autorise aucun socket Unix hérité. Un socket créé dans le sandbox (étiquette `unpriv_bwrap`) est refusé (`apparmor="DENIED" operation="file_inherit" ... peer="unpriv_bwrap"`), et `fusermount3` répond « file descriptor 3 is not a socket ». La méthode de GNOME Builder et de RClone Manager échoue donc telle quelle sur cette version.

**Contournement retenu** : un **relais côté hôte**. Le wrapper lance, via `flatpak-spawn --host`, un petit programme qui crée lui-même une paire de sockets sur l'hôte, exécute `fusermount3` avec, reçoit le descripteur `/dev/fuse` et le renvoie à rclone par le socket transmis depuis le sandbox. Aucune modification du système n'est nécessaire.

- Prototype (`poc/flatpak-fuse/`) : relais en Python lancé par `python3` de l'hôte. Montage, lecture, écriture depuis l'hôte et démontage validés le 2026-10-30.
- Version finale : un petit binaire Rust `nuage-fuse-relay`, livré dans le Flatpak et exécuté sur l'hôte à partir du chemin de l'application (lu dans `/.flatpak-info`), pour ne pas dépendre de Python sur l'hôte.
- À signaler à Ubuntu (Launchpad, paquet `fuse3`) : le profil casse aussi GNOME Builder.

Référence : `fusermount-wrapper.sh` dans `github.com/flathub/org.gnome.Builder` et `github.com/flathub/io.github.zarestia_dev.rclone-manager`.

**Conséquences, à documenter dans le README :**

- **Le sandbox est percé** : la permission `--talk-name=org.freedesktop.Flatpak` permet d'exécuter des commandes sur l'hôte. Acceptable pour une v1 distribuée en bundle ; à justifier pour Flathub (Builder l'a obtenue).
- **Usage limité** : le wrapper ne relaie **que** `fusermount3`. Aucun autre appel à `flatpak-spawn --host` dans l'application.
- **Dépendance hôte** : `fuse3` (installé par défaut sur Ubuntu).
- **Même chemin des deux côtés** : c'est le `fusermount3` de l'hôte qui monte, avec le chemin que lui passe rclone ; le dossier de montage doit donc exister au même chemin dans le sandbox et sur l'hôte. Comme l'utilisateur choisit librement son dossier, on demande `--filesystem=home`.

## Architecture

### Modularité

Tout ce qui dépend d'un fournisseur est isolé derrière un trait `Provider`. Comme rclone fait les entrées/sorties pour tous les backends, **un fournisseur est essentiellement déclaratif** : type de backend rclone, flux d'authentification, paramètres, capacités, options recommandées. Le moteur, les interfaces et la CI ne connaissent que l'abstraction.

> **Ajouter un fournisseur = ajouter un module, sans toucher à `engine`, `ui`, `tray` ni `cli`.**

### Processus

```
nuaged  (démon : engine + rclone rcd)
   ▲  socket Unix, JSON (Command / Event)
   ├─ nuage-gtk   application de bureau : fenêtre + icône de barre système
   └─ nuage       CLI (humains, scripts, agents)
```

- Le démon possède tout l'état : comptes, montages, exclusions, épingles. Les clients ne parlent jamais à rclone.
- Fermer ou quitter l'application GTK n'arrête pas forcément la sync (voir « Quit » dans [Interface](#interface-gnome--libadwaita)).
- Un seul démon par utilisateur ; un second lancement détecte le premier par le socket et s'arrête.

### Workspace Cargo

```
nuage/
├─ Cargo.toml                      (workspace)
├─ crates/
│  ├─ core/        nuage-core       modèle de domaine, traits, Command/Event (aucune dépendance GTK)
│  ├─ ipc/         nuage-ipc        protocole socket Unix : sérialisation, client, serveur
│  ├─ rclone-rc/   nuage-rclone     client typé de l'API rc + supervision du processus rcd
│  ├─ providers/   nuage-providers  trait Provider + registre + un module par fournisseur
│  │   └─ src/{gdrive.rs, onedrive.rs, dropbox.rs, local.rs, …}   (une feature Cargo par fournisseur)
│  ├─ engine/      nuage-engine     comptes, montages, exclusions, épinglage, agrégation d'état
│  ├─ fs/          nuage-fs         système de fichiers FUSE superposé (relais + exclusions en v1, épinglage en v1.1)
│  ├─ daemon/      nuaged           binaire : engine + serveur ipc
│  ├─ cli/         nuage            binaire : CLI (clap), client ipc
│  ├─ tray/        nuage-tray       icône StatusNotifierItem (ksni)
│  ├─ ui/          nuage-ui         GTK4 + libadwaita : navigateur, mise en route, réglages
│  └─ gtk/         nuage-gtk        binaire : assemble ui + tray, client ipc
├─ build-aux/flatpak/              manifeste, fusermount-wrapper.sh, cargo-sources.json
├─ build-aux/native/               unit systemd, configuration cargo-deb, script du tarball
├─ data/                           .desktop, metainfo.xml, icônes, schémas
├─ docs/ARCHITECTURE.md
├─ docs/ADDING_A_PROVIDER.md
├─ docs/CLI.md
├─ .github/workflows/              CI
└─ README.md
```

**Règles de dépendance** (vérifiées en CI) :

| Crate | Peut dépendre de |
|---|---|
| `core` | aucune crate du projet |
| `ipc` | `core` |
| `providers` | `core` |
| `rclone-rc` | `core` |
| `engine` | `core`, `rclone-rc`, `providers`, `fs` |
| `daemon` | `engine`, `ipc`, `core` |
| `cli`, `ui`, `tray`, `gtk` | `core`, `ipc` (+ `ui`, `tray` pour `gtk`) ; **jamais** `engine` ni `rclone-rc` |

`daemon` et `cli` ne doivent tirer **aucune dépendance GTK** (vérifié en CI avec `cargo tree`), pour que le paquet headless s'installe sur un serveur nu.

### Protocole IPC (`ipc`)

- Socket Unix `$XDG_RUNTIME_DIR/nuage/nuaged.sock`, droits `0600`. Dans le Flatpak : `$XDG_RUNTIME_DIR/app/io.github.felixilgatto.Nuage/nuaged.sock`, partagé avec l'hôte, pour que la CLI de l'hôte puisse aussi s'y connecter.
- JSON délimité par des retours à la ligne : requêtes `Command` avec réponse, plus un abonnement au flux d'`Event`.
- Poignée de main avec numéro de version du protocole ; un client trop ancien ou trop récent reçoit une erreur claire.

### Modèle de domaine (`core`)

```rust
AccountId
Account   { id, provider_id, display_name, remote_name, mount_point, cache_limit }
RemotePath { account, path }
Entry     { name, path, kind: EntryKind /* File | Folder */, size, modified, mime, hash,
            native_doc: Option<NativeDoc> }      // Google Docs : lien web
Quota     { total, used, free, trashed }         // tous optionnels : tous les backends ne les donnent pas
SyncState  = UpToDate | Syncing { uploads, queued } | Paused { queued } | Error { … } | Stopped | Offline
Capabilities                                      // voir « Trait Provider »
Exclusion { account, rule: ExclusionRule /* Path(RemotePath) | Glob(String) */, created_at }
Pin       { account, path, kind: PinKind /* File | FolderRecursive */, pinned_at }   // v1.1
PinState   = Downloading | Available | ModifiedLocally | Conflict | Error             // v1.1
Command   // client → démon
Event     // démon → clients
```

`Account.mount_point` est le **dossier choisi par l'utilisateur** ; il ne doit pas être confondu avec le point de montage interne de rclone (voir [Mode hors ligne](#mode-hors-ligne-v11)).

### Trait `Provider`

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;                    // "gdrive"
    fn display_name(&self) -> &str;                  // "Google Drive"
    fn icon_name(&self) -> &str;
    fn rclone_backend(&self) -> &'static str;        // "drive"
    fn auth(&self) -> AuthSpec;                      // OAuthBrowser | Credentials(form) | None
    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)>; // ex. ("scope", "drive")
    fn static_capabilities(&self) -> Capabilities;   // complétées à l'exécution par operations/fsinfo
    fn recommended_vfs_opts(&self) -> VfsOptions;
    fn native_docs(&self) -> Option<NativeDocPolicy>; // Google Docs → raccourcis web
}
```

- **Registre** : `ProviderRegistry`, construit à partir des features Cargo activées.
- **Capacités réelles** = capacités statiques fusionnées avec `operations/fsinfo` (rclone y expose les `Features` du backend : `About`, `PublicLink`, `Copy`, `Move`, `PurgeTrash`…). **Les interfaces masquent ou refusent les actions selon `Capabilities`.**
- **Fournisseur `local`** (backend rclone `local` ou `memory`) : pour les tests et la CI, sans identifiants.
- **v1** : `gdrive` complet + `local`. `onedrive` peut être ajouté tôt pour valider l'abstraction (il est surtout déclaratif).

### Moteur (`engine`, dans `nuaged`)

```
nuaged
 ├─ RcdSupervisor     lance et surveille `rclone rcd` (127.0.0.1, port et identifiants aléatoires)
 ├─ AccountManager    création, renommage, suppression, reconnexion des comptes
 ├─ MountManager      mount/mount, mount/unmount, remontage au démarrage, nettoyage des orphelins
 ├─ UploadControl     pause / reprise des uploads, par compte ou globale
 ├─ ExclusionManager  règles d'exclusion par compte → filtres rclone du montage
 ├─ StatusPoller      interroge vfs/stats, core/stats, operations/about ; publie des Event
 ├─ FileService       opérations sur les fichiers (list, mkdir, move, copy, delete, upload, download, lien)
 └─ PinManager        épinglage hors ligne (v1.1)
```

Dans le Flatpak, `/app/bin` contient `rclone` (version épinglée), le wrapper `fusermount3`, `nuaged`, `nuage` et `nuage-gtk`.

**Cycle de vie :**

- rclone est un processus enfant du démon.
- Au démarrage : nettoyer les montages orphelins laissés par un crash, puis remonter les comptes.
- À l'arrêt (`SIGTERM`, `nuage daemon stop`, « Quit ») : démonter tous les comptes, puis `core/quit`. Le comportement s'il reste des uploads est décrit dans [Quit](#quit).

### Endpoints rc utilisés

| Besoin | Endpoint | Remarque |
|---|---|---|
| Démon vivant | `core/version` | health check, version minimale |
| Capacités | `operations/fsinfo` | features du backend |
| Monter | `mount/mount` | `fs`, `mountPoint`, `vfsOpt` (`CacheMode=full`, `CacheMaxSize`, `CacheMaxAge`, `WriteBack` très long), `_filter` (`ExcludeRule` / `FilterRule`) ; tout en JSON |
| Lister / démonter | `mount/listmounts`, `mount/unmount` | |
| État de sync | `vfs/stats` | `fs` **obligatoire** dès que plusieurs VFS sont actifs ; `diskCache.{uploadsInProgress, uploadsQueued, erroredFiles, bytesUsed, outOfSpace}` ; `bytesUsed` n'est rafraîchi qu'au poll du cache (60 s par défaut) |
| File d'upload | `vfs/queue` | `{"queue":[{name, id, size, expiry, tries, delay, uploading}]}` ; badge « uploading » par fichier |
| Pause des uploads | `vfs/queue` + `vfs/queue-set-expiry` | voir [Pause](#état-de-synchronisation-pause-et-quota) |
| Quota | `operations/about` | `total`, `used`, `free`, `trashed` ; les champs absents sont omis ; vérifier `Features.About` avant l'appel (le backend `memory` renvoie une erreur 500) |
| Rafraîchir | `vfs/refresh` | |
| Navigation | `operations/list`, `operations/stat` | |
| Opérations fichiers | `operations/mkdir`, `movefile`, `copyfile`, `deletefile`, `purge`, `uploadfile`, `publiclink` | `_async=true` + `job/status` pour les opérations longues |
| Transferts | `core/stats` | débit, progression |
| Épinglage (v1.1) | `operations/copyfile`, `operations/stat`, `operations/list` (hashes, modtime) | téléchargement, renvoi, détection des changements distants |
| Comptes | `config/create`, `config/update`, `config/listremotes`, `config/get`, `config/delete` | machine à états non interactive (`opt: {nonInteractive, obscure, continue, state, result}`) ; `config/create` enregistre le remote avant la fin des questions : le supprimer si l'utilisateur abandonne |
| OAuth (GUI) | `config/create` avec `_async`, `config/oauthstatus`, `config/oauthstop` | `config_auth_no_browser: "true"` puis `oauthstatus` donne `authUrl` à ouvrir via OpenURI ; `oauthstop` annule |
| Arrêt | `core/quit` | |

Les noms de champs et d'options évoluent d'une version de rclone à l'autre : les vérifier sur la version embarquée. Un test d'intégration doit échouer si un champ attendu disparaît lors d'une montée de version.

### Ce que veut dire « à jour »

En mode montage, rien n'est copié à l'avance. **« À jour » = aucune écriture locale en attente d'upload.** Les changements faits ailleurs apparaissent au prochain rafraîchissement (polling du backend par rclone) ou via « Refresh ».

## Fonctionnalités

### Comptes et authentification

- La mise en route (GUI) ou `nuage account add` (CLI) liste les fournisseurs du registre, puis suit l'`AuthSpec` du fournisseur choisi.
- **Nom par défaut d'un compte** : le nom du fournisseur (« Google Drive »), suffixé s'il est déjà pris (« Google Drive 2 »). Il sert aussi au dossier par défaut. Renommable ensuite ; l'adresse e-mail est affichée en sous-titre quand le fournisseur la donne.
- Token expiré ou révoqué → état ⚠ et action « Reconnect » (`rclone config reconnect`, ou `nuage account reconnect`).

**Suppression d'un compte** (après confirmation ; en CLI, `--yes` obligatoire) : démontage, suppression du remote rclone, du cache, des épingles et du dossier de montage s'il est vide. **Rien n'est supprimé sur le cloud.** Refusée tant qu'il reste des uploads en attente, sauf avec `--force`.

**OAuth sur le bureau** : `rclone config create <nom> <backend> …` ; rclone ouvre son serveur local `127.0.0.1:53682` et le navigateur (via le portail OpenURI depuis le sandbox). Cas d'erreur à gérer : port occupé, annulation, timeout, navigateur qui ne s'ouvre pas (afficher l'URL à copier).

**OAuth en headless** (deux méthodes, documentées dans le README) :

1. **URL + tunnel SSH** (par défaut sans navigateur local) : la CLI affiche l'URL d'autorisation et attend sur `127.0.0.1:53682`. L'utilisateur ouvre un tunnel (`ssh -L 53682:localhost:53682 serveur`) et ouvre l'URL sur son poste. Côté rclone : paramètre `config_auth_no_browser: "true"` (le drapeau `--auth-no-open-browser` n'existe que pour `rclone authorize`).
2. **Token généré ailleurs** (entièrement scriptable) : `rclone authorize drive <client_id> <client_secret>` sur un poste avec navigateur, puis `nuage account add gdrive --client-id … --client-secret … --token-file token.json` (ou `--token '<json>'`) sur le serveur. Côté rclone, un seul `config/create` suffit en fournissant `token`, `config_refresh_token: "false"`, `config_change_team_drive: "false"`.

**Google Drive :**

- v1 : **« Mon Drive » uniquement**. Drive partagés et « Partagés avec moi » plus tard.
- scope `drive` : accès aux fichiers uniquement, aucun autre service Google ;
- **client OAuth perso obligatoire, avec un parcours guidé.** Le client partagé de rclone « cessera de fonctionner courant 2026 » (avertissement de rclone 1.75), et un client propre à Nuage imposerait une vérification Google + un audit CASA (scope `drive` « restricted »). Chaque utilisateur crée donc son client GCP (type « Desktop app ») :
  - GUI : un écran guidé avant l'authentification (étapes numérotées, liens directs vers la console GCP, champs client ID / secret à coller, vérification immédiate) ;
  - CLI : `--client-id` / `--client-secret` obligatoires pour `gdrive`, avec un message qui renvoie au guide du README s'ils manquent ;
  - le client est mémorisé globalement : les comptes Google suivants ne repassent pas par cet écran ;
- `drive` est un scope « restricted » : en statut *Testing*, les refresh tokens expirent au bout de 7 jours. Le guide fait publier le projet GCP en *Production* (non vérifié, suffisant pour un usage personnel) ;
- **Google Docs/Sheets/Slides natifs** : présentés comme des **raccourcis web** dans le dossier monté, au format d'export rclone **`link.html`** (page qui redirige vers le document). Le format `desktop` s'ouvre dans l'éditeur de texte depuis Nautilus, donc il est écarté. Un double-clic ouvre `link.html` avec l'application HTML par défaut (normalement le navigateur). Les fichiers lien sont en lecture seule (rclone refuse de les modifier). Pas d'export docx/xlsx/pptx, donc ni doublons ni perte de mise en forme. Dans le navigateur de Nuage, ils sont signalés et s'ouvrent directement dans le navigateur web.

**OneDrive** (quand il sera ajouté) : scopes `Files.ReadWrite.All` + `offline_access` uniquement (défaut rclone) ; app Azure perso facultative.

### Montage et dossier de montage

- Un dossier de montage **par compte**, choisi par l'utilisateur (`gtk::FileDialog` ou `--mount-point`). Défaut : `~/Cloud/<nom-du-compte>`.
- Validations : dossier vide ou inexistant (on le crée), pas déjà utilisé par un autre compte, pas imbriqué dans un autre point de montage, pas dans le cache.
- **Flatpak, sous le home** : couvert par `--filesystem=home`. Le portail FileChooser renvoie alors le vrai chemin. Rejeter tout chemin en `/run/user/*/doc/` avec un message clair.
- **Flatpak, hors du home** (ex. `/mnt/data/Drive`) : l'application le détecte et affiche la commande à lancer : `flatpak override --user --filesystem=/mnt/data/Drive io.github.felixilgatto.Nuage`.
- **Natif** : aucune restriction au-delà des droits de l'utilisateur.
- Changer de dossier démonte l'ancien et remonte au nouveau.

### Cache local

- Fichiers téléchargés à la demande, dans le cache VFS de rclone (`CacheMode=full`).
- **Limite par compte**, **20 Go par défaut**. Au-delà, les fichiers les plus anciens sont évincés. L'interface affiche aussi le total de tous les comptes.
- Âge maximal du cache : défaut raisonnable, modifiable dans les réglages.

### État de synchronisation, pause et quota

**État agrégé** sur tous les comptes, affiché par l'icône de la barre système et par `nuage status` :

| Icône | État | Condition |
|---|---|---|
| ✓ | à jour | aucune écriture locale en attente d'upload |
| ↻ | sync en cours | n fichiers en upload ou en file d'attente |
| ⚠ | problème | erreur, compte déconnecté ou fichiers en échec |
| ⏸ | en pause | uploads suspendus ou montage arrêté |

**Pause** : suspend les **uploads**, par compte ou pour tous. Le dossier reste monté et utilisable ; les écritures s'accumulent dans le cache et partent à la reprise. La pause survit à un redémarrage du démon.

Mécanisme (vérifié sur rclone 1.75.2) : chaque compte est monté avec un `WriteBack` très long (ex. `8760h`), donc rclone n'envoie rien de lui-même. Quand le compte n'est pas en pause, `UploadControl` interroge `vfs/queue` toutes les 1 à 2 secondes et libère chaque nouvel élément avec `vfs/queue-set-expiry` (`expiry: -1e9`). En pause, il ne libère rien. L'état de pause est dans la config TOML de Nuage. Les écritures en attente survivent à un arrêt de rclone et sont remises en file au remontage. `core/bwlimit` est écarté : il est global et ralentit sans jamais suspendre.

**Quota** par compte : utilisé / total / libre / corbeille, avec un pourcentage. Champs absents si le backend ne les fournit pas.

### Navigateur de fichiers unifié (GUI)

**Un seul design pour tous les fournisseurs.** Les composants sont génériques et ne connaissent que `Entry`, `Capabilities` et les `Command` de fichiers. La navigation passe par l'API rc, **indépendamment du montage** : elle fonctionne même compte démonté.

**Structure** (`AdwNavigationSplitView`) :

- **Barre latérale** : les comptes, chacun avec l'icône du fournisseur et une barre de quota. En v1.1, une entrée « Available offline » liste tous les éléments épinglés.
- **Contenu** : fil d'Ariane, vue liste (`GtkColumnView` : nom, taille, date, état) ou vue grille (`GtkGridView`) ; le choix est mémorisé.
- **Barre d'en-tête** : retour, chemin, filtre du dossier courant, nouveau dossier, envoyer des fichiers, menu.

**Actions**, affichées seulement si la capacité existe :

- ouvrir, télécharger vers…, envoyer (bouton ou glisser-déposer depuis Nautilus) ;
- renommer, déplacer, copier, nouveau dossier ;
- supprimer (vers la corbeille si supportée, sinon avec confirmation) ;
- copier le lien de partage, propriétés, « Open in Files » (dossier monté) ;
- « Exclude from sync » / « Include in sync » (voir [Exclusions](#exclusions-de-la-synchronisation)) ;
- v1.1 : « Keep offline » / « Remove offline copy », avec icône cadenas.

**Ouvrir un fichier** : si le compte est monté, ouvrir son chemin dans le montage via le portail OpenURI (application hôte par défaut) ; sinon, le télécharger dans un dossier temporaire puis l'ouvrir. Un Google Doc natif s'ouvre dans le navigateur web.

**Badges par fichier** :

| Badge | Version | Source |
|---|---|---|
| Uploading | v1 | `vfs/queue` |
| Not synced (élément exclu, grisé) | v1 | règles d'exclusion |
| Online only (hors connexion, non épinglé, grisé) | v1 | listings en cache |
| Available offline / Modified locally / Conflict / Error | v1.1 | `PinState` |

**Comportement** : gros dossiers chargés de façon paginée ou progressive ; aucun appel bloquant sur le thread GTK ; erreurs affichées dans un `AdwToast`.

### Exclusions de la synchronisation

**Besoin.** Par compte, l'utilisateur choisit ce qu'il **ne veut pas synchroniser** :

- des fichiers ou dossiers distants précis (ex. un gros dossier « Archives ») ;
- des motifs valables pour tout le compte (ex. `*.tmp`, `~$*`, `.~lock.*`, `node_modules/`, `.git/`).

Un élément exclu n'est **ni téléchargé, ni mis en cache, ni envoyé**, et n'apparaît pas dans le dossier monté. Il reste intact sur le cloud.

**Règles :**

- Motifs proposés par défaut (désactivables) : fichiers temporaires et verrous Office/LibreOffice (`~$*`, `.~lock.*#`, `*.tmp`, `.goutputstream-*`).
- **Épingler et exclure s'excluent mutuellement.** Exclure un élément épinglé demande confirmation, puis le désépingle (après renvoi de ses modifications). Un élément exclu ne peut pas être épinglé (action masquée en GUI, erreur en CLI).
- Persistance : config TOML de l'application, par compte (pas dans `rclone.conf`).

**Mise en œuvre :**

Les filtres rclone seuls ne suffisent pas (vérifié sur rclone 1.75.2) : `_filter` sur `mount/mount` masque bien les éléments distants exclus et empêche de les lire, mais un fichier exclu **créé localement** dans le montage est accepté **et envoyé**. D'où deux couches, dès la v1 :

- **Côté distant : filtres rclone** sur le montage rclone caché (`_filter` → `ExcludeRule` / `FilterRule`). Les éléments distants exclus ne sont ni listés ni téléchargés. Modifier une exclusion **remonte le montage rclone caché** (pas le dossier de l'utilisateur, que `nuage-fs` garde en place), une fois la file d'upload vide ; sinon on demande confirmation (`--yes` en CLI).
- **Côté local : `nuage-fs`** (voir [Système de fichiers superposé](#système-de-fichiers-superposé-nuage-fs)). Un fichier créé ou renommé localement qui correspond à une exclusion n'est jamais transmis à rclone : il est **gardé uniquement en local** (dossier `local-only`, voir [Sécurité et stockage](#sécurité-et-stockage)) et reste visible à son chemin dans le dossier monté.

**Interface :**

- Navigateur : « Exclude from sync » / « Include in sync » dans le menu contextuel. Les éléments exclus restent visibles dans le navigateur (qui ne passe pas par les filtres), grisés avec le badge « Not synced », pour pouvoir les réinclure.
- Réglages du compte : liste « Excluded from sync » (chemins et motifs), ajout d'un motif, suppression d'une règle.
- CLI : `nuage exclude add|remove|list`.
- Rien dans la mise en route : par défaut, tout est synchronisé.

### Système de fichiers superposé (`nuage-fs`)

Dès la v1, l'utilisateur ne voit **pas** directement le montage rclone. Le démon monte son propre système de fichiers FUSE (crate `fuser`) au point de montage de l'utilisateur, au-dessus d'un montage rclone caché :

```
~/Cloud/Google Drive                  ← dossier vu par l'utilisateur (nuage-fs)
   ├─ élément exclu, créé localement  → dossier local-only (v1)
   ├─ élément épinglé                  → magasin d'épingles local (v1.1)
   └─ tout le reste                    → relayé au montage rclone caché
                                          data/mounts/<compte>  (cache VFS limité, filtres d'exclusion)
```

**v1 : relais + exclusions.**

- Relais transparent de toutes les opérations vers le montage rclone caché (lecture, écriture, renommage, attributs, listing), sans régression de performance perceptible ni de comportement par rapport à un montage rclone direct.
- Interception des créations et renommages qui correspondent à une exclusion : ces fichiers vont dans `local-only` et ne sont jamais vus par rclone. Leur listing est fusionné avec celui de rclone.
- Le montage utilisateur reste en place quand le montage rclone caché est remonté (changement d'exclusions, reconnexion) : les accès en cours attendent ou échouent proprement, sans que le dossier disparaisse.
- Dans le Flatpak, `nuage-fs` passe par le même wrapper `fusermount3` que rclone : aucune permission supplémentaire.

**v1.1 : épinglage hors ligne**, décrit ci-dessous.

C'est la brique la plus délicate du projet. Elle doit être couverte par des tests d'intégration dédiés (relais, exclusions, remontage) avec le fournisseur `local`, et passer une suite de tests de système de fichiers (ex. `pjdfstest` ou équivalent, au minimum les cas lecture/écriture/renommage/suppression).

### Mode hors ligne (v1.1)

**Besoin.** L'utilisateur **épingle** un fichier ou un dossier. L'élément reste **au même chemin dans le dossier monté** et s'ouvre et se modifie **sans réseau**, y compris **après un redémarrage hors ligne**. C'est l'équivalent du « Disponible hors connexion » de OneDrive ou de Google Drive pour ordinateur. Les éléments épinglés sont **hors du cache** : jamais évincés, quelle que soit la limite.

**Pourquoi le cache VFS de rclone ne suffit pas :**

- il n'a **pas d'épinglage** : un fichier peut être évincé dès que le cache atteint sa limite ;
- le cache des répertoires est **en mémoire** : après un redémarrage sans réseau, rclone ne peut plus lister les dossiers et les fichiers en cache deviennent inaccessibles par leur chemin ;
- le montage rclone peut **refuser de démarrer** si le backend est injoignable (rafraîchissement du token) ;
- les modifications hors ligne sont renvoyées en « le dernier qui écrit gagne », sans détection de conflit.

**Conception retenue : l'épinglage est ajouté à `nuage-fs`** (voir [Système de fichiers superposé](#système-de-fichiers-superposé-nuage-fs)). Les éléments épinglés sont lus et écrits dans un magasin local ; le reste continue d'être relayé à rclone.

| Situation | Comportement |
|---|---|
| Lecture d'un fichier épinglé | depuis le magasin d'épingles |
| Lecture d'un fichier non épinglé | relayée à rclone ; hors ligne → erreur propre (`EHOSTUNREACH`), l'élément reste listé grâce aux métadonnées |
| Écriture sur un fichier épinglé | écrite localement, marquée « modifiée », renvoyée par `operations/copyfile` dès le retour du réseau (sauf en pause) |
| Écriture sur un fichier non épinglé | relayée à rclone (son cache d'écriture différée gère l'upload) |
| Changement distant d'un élément épinglé | détecté par vérification périodique (`operations/list` avec hash et date, intervalle réglable), nouvelle version téléchargée |
| Changement des deux côtés depuis l'état de référence | **conflit** : on garde les deux versions (`report (conflict 2026-10-29 18-45).docx`), jamais de perte, et une notification |
| Nouveau fichier dans un dossier épinglé (local ou distant) | épinglé automatiquement (l'épinglage d'un dossier est récursif) |
| Démarrage sans réseau | `nuage-fs` se monte à partir des métadonnées même si rclone échoue, puis bascule sur rclone dès qu'il est disponible |
| Désépinglage | la copie locale est libérée une fois les modifications locales renvoyées |

**Stockage :**

- **Magasin d'épingles** : copies complètes, hors de la limite du cache VFS.
- **Métadonnées** (SQLite, `rusqlite`) : liste des épingles ; pour chaque fichier épinglé, son état de référence (taille, date, hash distant, hash local) ; **les listings de dossiers** (au minimum ceux des éléments épinglés et de leurs parents) pour que l'arborescence reste navigable hors ligne et après un redémarrage.
- **Espace** : la taille du magasin est affichée à part du cache (« Offline: 4.2 GB »), avec un avertissement optionnel au-delà d'un seuil choisi par l'utilisateur.

C'est le jalon le plus lourd du projet : une partie de Nuage devient un vrai client de sync (sur les seuls éléments épinglés). Les transferts restent délégués à rclone.

**Interface :**

- Navigateur : « Keep offline » / « Remove offline copy » (menu contextuel et bouton cadenas), badges d'état, entrée « Available offline » dans la barre latérale.
- Réglages du compte : liste « Available offline » des éléments épinglés, avec leur taille et l'action « Remove offline copy ».
- Tray : nombre d'éléments épinglés en attente d'envoi ou en conflit.
- Réglages : intervalle de vérification, seuil d'avertissement d'espace.
- CLI : `nuage pin add|remove|list`.

## CLI (`nuage`)

La CLI est l'interface du mode headless et un outil de script sur le bureau. Elle doit pouvoir être **installée et utilisée de bout en bout par un agent** (ex. Claude Code) dans une session SSH.

**Règles :**

- **Jamais d'invite interactive obligatoire** : tout passe par des arguments. Les confirmations deviennent `--yes`. Si l'entrée standard n'est pas un terminal et qu'une confirmation manque, la commande échoue avec un message qui indique l'option à ajouter.
- **`--json` sur toutes les commandes** : sortie stable et documentée, pour les scripts. Sans `--json`, sortie lisible et sans couleur si la sortie n'est pas un terminal.
- **Codes de retour documentés** : `0` succès, `1` erreur, `2` usage incorrect, `3` démon injoignable, `4` authentification requise (liste complète dans `docs/CLI.md`).
- Les comptes se désignent par leur nom ; les chemins distants s'écrivent `<compte>:<chemin>`.
- Démon absent : message clair avec la commande pour le démarrer (`systemctl --user start nuaged`).

**Commandes (v1) :**

```
nuage status [--watch]                         état global, par compte, uploads, quota, cache
nuage account add <fournisseur> [--name N] [--mount-point P]
                     [--token JSON | --token-file F] [--client-id ID --client-secret S] [--no-browser]
nuage account list | rename <compte> <nom> | reconnect <compte> | remove <compte> --yes [--force]
nuage mount <compte> | unmount <compte>
nuage pause [<compte>] | resume [<compte>]
nuage refresh [<compte>]
nuage ls | mkdir | mv | cp | rm | upload | download | link   opérations sur <compte>:<chemin>
nuage exclude add <compte> (<chemin> | --glob <motif>) | remove | list
nuage config get | set <clé> <valeur>                     ex. cache limit, âge du cache
nuage daemon status | stop
nuage pin add | remove | list                             (v1.1)
```

## Interface (GNOME / libadwaita)

Rappel : [interface minimaliste](#principes). Valeurs par défaut jamais demandées pendant la mise en route : limite de cache (20 Go), âge du cache, intervalle de vérification hors ligne, exclusions.

### Mise en route (premier lancement et « Add account »)

Objectif : **compte connecté et synchronisation lancée sans quitter la fenêtre, avec le minimum d'écrans** : 4 pour un premier compte Google, 3 ensuite ou pour un fournisseur qui n'a pas besoin de client OAuth. Une seule fenêtre (`AdwNavigationView`, bouton retour à chaque étape).

1. **Choisir un fournisseur** : la liste des fournisseurs du registre (icône + nom). Un clic passe à l'étape suivante. Rien d'autre à l'écran.
2. **Créer son client Google** (premier compte Google seulement) :
   - étapes numérotées et courtes : créer un projet, activer l'API Drive, configurer l'écran de consentement et le **publier en Production**, créer des identifiants « Desktop app » ; chaque étape a un bouton qui ouvre la bonne page de la console GCP ;
   - deux champs « Client ID » et « Client secret » ; le bouton « Continue » reste désactivé tant qu'ils n'ont pas le bon format ;
   - le client est mémorisé : les comptes Google suivants sautent cet écran (modifiable dans les réglages).
3. **S'authentifier** :
   - le navigateur s'ouvre automatiquement (OAuth rclone avec le client de l'utilisateur) ;
   - l'écran affiche un spinner « Waiting for sign-in in your browser… », un bouton « Cancel » et un lien « Copy sign-in link » au cas où le navigateur ne s'ouvre pas ;
   - on passe automatiquement à l'étape suivante dès que le token est reçu ;
   - si Google refuse le client (identifiants faux, projet en *Testing*), retour à l'étape 2 avec un message qui dit quoi corriger ;
   - pour un fournisseur à identifiants (`AuthSpec::Credentials`), cet écran est le formulaire.
4. **Lancer la synchronisation** :
   - **Dossier** : un `AdwActionRow` affiche le dossier pré-rempli (`~/Cloud/<nom-du-compte>`) avec un bouton « Choose… » (`gtk::FileDialog`). Validations de [Montage et dossier de montage](#montage-et-dossier-de-montage) ; erreur affichée sous la ligne ; bouton principal désactivé tant que le dossier est invalide.
   - **« Start automatically at login »** (`AdwSwitchRow`), **activé par défaut**. Il déclenche la demande au portail Background ; si le portail refuse, l'interrupteur se désactive et un toast explique pourquoi. C'est un réglage global (pas par compte), affiché seulement lors de l'ajout du premier compte, modifiable ensuite dans les réglages.
   - Bouton principal **« Start sync »** (`suggested-action`) : crée le dossier si besoin et monte le compte. La fenêtre se ferme, l'application continue dans la barre système et envoie la notification « Google Drive is syncing to ~/Cloud/Google Drive » avec une action « Open folder ». **Si aucune barre système n'est disponible** (GNOME sans extension AppIndicator), la fenêtre affiche la fenêtre principale au lieu de se fermer.

Aux lancements suivants, si au moins un compte existe, l'application lancée par l'utilisateur ouvre la fenêtre principale ; démarrée à la connexion (`--background`), elle reste dans la barre système, fenêtre fermée (décidé au jalon 8).

### Fenêtre principale

Minimale : la liste des comptes (icône, nom, état ✓/↻/⚠/⏸, barre de quota, bouton « Open folder ») et un bouton « + » pour ajouter un compte. Le navigateur s'ouvre depuis un compte ; il n'est jamais affiché d'office. Les réglages sont dans le menu principal. Fermer la fenêtre n'arrête pas la sync.

### Menu de l'icône de barre système

- État global
- Par compte : quota, taille du cache local, taille hors ligne (v1.1)
- « Open Nuage » (fenêtre principale)
- « Open folder » (dossier monté)
- « Refresh »
- « Pause uploads » / « Resume uploads »
- « Preferences »
- « Add account »
- « Quit »

Sur Ubuntu, l'extension AppIndicator est active par défaut et l'icône s'affiche. Sur un GNOME sans extension, la fenêtre principale doit suffire à tout faire.

### Quit

« Quit » arrête l'application et le démon (démontage puis arrêt de rclone). S'il reste des uploads en attente, le comportement se règle dans les réglages (« When quitting with pending uploads ») :

| Option | Effet |
|---|---|
| **Finish in background** (défaut) | la fenêtre et l'icône disparaissent ; le démon termine les uploads, puis démonte et s'arrête |
| Ask | dialogue « N files are still uploading » : Wait / Quit now |
| Quit now | arrêt immédiat ; les uploads restent dans le cache rclone et reprennent au prochain lancement |

En CLI, `nuage daemon stop` applique le même réglage ; `--now` force l'arrêt immédiat.

### Réglages (`AdwPreferencesDialog`)

- Global : démarrage automatique, comportement de « Quit », client OAuth, intervalle de vérification hors ligne (v1.1), seuil d'avertissement d'espace hors ligne (v1.1).
- Par compte : nom, dossier de montage, limite du cache, âge maximal du cache, exclusions, éléments épinglés (v1.1), suppression du compte.

### Notifications (portail Notification)

Erreur d'upload, token expiré, cache plein, conflit hors ligne (v1.1). En headless, ces événements apparaissent dans `nuage status` et dans le journal du service (`journalctl --user -u nuaged`).

## Sécurité et stockage

- `rclone.conf` est chiffré avec `RCLONE_CONFIG_PASS`. Le mot de passe est rangé dans le trousseau : portail Secret (`ashpd`) dans le Flatpak, Secret Service (`org.freedesktop.secrets`) en natif s'il est disponible.
- **Headless sans trousseau** : `rclone.conf` n'est pas chiffré mais protégé en `0600` (comme rclone par défaut). `nuage status` le signale et le README l'explique.
- API rc sur 127.0.0.1, port et identifiants aléatoires à chaque lancement, jamais `--rc-no-auth`.
- Socket IPC en `0600`, accessible au seul utilisateur.
- Aucun token ni mot de passe dans les logs (filtre `tracing`).

**Emplacements** (Flatpak : `~/.var/app/io.github.felixilgatto.Nuage/{config,cache,data}` ; natif : répertoires XDG `~/.config/nuage`, `~/.cache/nuage`, `~/.local/share/nuage`) :

| Contenu | Sous-dossier | Version |
|---|---|---|
| Config rclone (`RCLONE_CONFIG`) | `config/rclone/rclone.conf` (natif : `~/.config/nuage/rclone.conf`) | v1 |
| Config de l'application (TOML : comptes, exclusions, réglages) | `config/nuage/` (natif : `~/.config/nuage/`) | v1 |
| Cache VFS (`--cache-dir`) | `cache/rclone` | v1 |
| Montages rclone cachés | `data/mounts/<compte>` | v1 |
| Magasin d'épingles | `data/pins/<compte>/…` | v1.1 |
| Fichiers exclus gardés en local | `data/local-only/<compte>/…` | v1 |
| Métadonnées hors ligne (SQLite) | `data/` | v1.1 |

## Stack technique

| Rôle | Crate | Remarque |
|---|---|---|
| UI | `gtk4`, `libadwaita` (gtk-rs), sans relm4 | sous-classes GObject, fichiers `.ui` ou Blueprint |
| Tray | `ksni` | StatusNotifierItem + dbusmenu en pur Rust (GTK4 n'a pas d'AppIndicator) |
| Portails | `ashpd` | Background, OpenURI, Notification, Secret, FileChooser |
| CLI | `clap` | sous-commandes, `--json` |
| IPC | `tokio` (`UnixListener`), `serde_json` | JSON délimité par lignes |
| HTTP vers rc | `reqwest` (rustls) | |
| Async | `tokio` ; côté GTK, `async-channel` vers la boucle GLib | ne jamais bloquer le thread GTK |
| Sérialisation | `serde`, `serde_json`, `toml` | |
| FUSE superposé (`nuage-fs`) | `fuser` | v1 |
| Métadonnées hors ligne (v1.1) | `rusqlite` (feature `bundled`) | |
| i18n | `gettext-rs` | chaînes en anglais, prêtes à traduire |
| Erreurs | `thiserror` (crates), `anyhow` (binaires) | |
| Logs | `tracing` | journald quand lancé par systemd |
| Paquet `.deb` | `cargo-deb` | |

## Packaging

### Flatpak (bureau)

Manifeste YAML dans `build-aux/flatpak/` :

- runtime `org.gnome.Platform` (dernière version stable) ;
- extension `org.freedesktop.Sdk.Extension.rust-stable` ;
- dépendances Cargo vendorisées avec `flatpak-cargo-generator` (`cargo-sources.json`), build hors ligne ;
- `command: nuage-gtk`.

Modules :

1. `rclone` : binaire officiel avec sha256 épinglé (ou build Go via l'extension golang), installé dans `/app/bin`.
2. `fusermount-wrapper` : script installé en `/app/bin/fusermount3`.
3. `nuage` : `cargo build --release --offline` (`nuaged`, `nuage`, `nuage-gtk`), `.desktop`, icône, `metainfo.xml`.

`finish-args` (minimaux) :

```yaml
- --socket=wayland
- --socket=fallback-x11
- --device=dri
- --share=ipc
- --share=network
- --talk-name=org.freedesktop.Flatpak        # wrapper fusermount3 uniquement
- --talk-name=org.kde.StatusNotifierWatcher  # icône de barre système
- --talk-name=org.freedesktop.secrets        # repli si le portail Secret manque
- --filesystem=home                          # dossiers de montage choisis librement
```

Jamais de `--filesystem=host`.

### Natif (headless)

- **Tarball** `nuage-<version>-x86_64-linux.tar.gz` : `nuaged`, `nuage`, `rclone` (même version épinglée que le Flatpak), `nuaged.service` (unit systemd utilisateur), script `install.sh` qui installe dans `~/.local` sans `sudo`.
- **`.deb`** (`cargo-deb`) : mêmes binaires dans `/usr/bin` (rclone dans `/usr/lib/nuage/` pour ne pas entrer en conflit avec le rclone d'apt), unit dans `/usr/lib/systemd/user/`, dépendance `fuse3`.
- Binaires liés statiquement autant que possible (musl à évaluer), sans aucune dépendance GTK.
- Installation type, documentée dans le README :

```
sudo apt install ./nuage_<version>_amd64.deb        # ou : tar xf … && ./install.sh
systemctl --user enable --now nuaged
loginctl enable-linger "$USER"                         # pour tourner sans session ouverte
nuage account add gdrive --token-file token.json --mount-point ~/Cloud/Drive
nuage status
```

## CI (GitHub Actions)

Workflows dans `.github/workflows/`, sur le dépôt `felixilgatto/nuage`. **Aucun identifiant cloud en CI** : les tests utilisent le fournisseur `local`/`memory`.

### `ci.yml` — à chaque push et pull request

**Job `check`** (runner `ubuntu-24.04` avec `libgtk-4-dev libadwaita-1-dev`, ou conteneur du SDK GNOME pour avoir les mêmes versions que le Flatpak) :

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace` : tests unitaires + tests d'intégration contre un vrai `rclone rcd` (même version que dans les paquets) avec le fournisseur `local` : création de compte, montage FUSE (disponible sur les runners Ubuntu), `vfs/stats`, pause/reprise, navigation, opérations fichiers, exclusions ; en v1.1, scénarios hors ligne d'`nuage-fs`.
- **Tests de bout en bout de la CLI** : démarrer `nuaged`, piloter un scénario complet avec `nuage … --json` (ajout de compte `local`, montage, écriture, `status`, pause, exclusion, suppression), vérifier sorties JSON et codes de retour.
- `cargo deny check` (licences compatibles MIT, avis de sécurité, doublons) via `EmbarkStudios/cargo-deny-action`.
- Règles de dépendance entre crates : script sur `cargo metadata` qui échoue si le [tableau des dépendances](#workspace-cargo) n'est pas respecté, ou si `daemon`/`cli` tirent une dépendance GTK.
- Cache Cargo : `Swatinem/rust-cache`.

**Job `native`** : build de `nuaged` et `nuage` sans GTK sur une image Ubuntu minimale, production du tarball et du `.deb` en artefacts, puis installation du `.deb` dans un conteneur propre et smoke test (`nuage --version`, démarrage du démon, `nuage status --json`).

**Job `flatpak`** (conteneur `ghcr.io/flathub-infra/flatpak-github-actions:gnome-<version>`, action `flatpak/flatpak-github-actions/flatpak-builder`) :

- vérifie que `cargo-sources.json` est à jour (régénération puis `git diff --exit-code`) ;
- construit le manifeste et publie le bundle `nuage.flatpak` en artefact ;
- lance `flatpak-builder-lint` sur le manifeste et le dépôt construit (AppStream, permissions). En v1, les avertissements sur `org.freedesktop.Flatpak` et `home` sont tolérés et documentés.

### `release.yml` — sur tag `v*`

- Rejoue les builds Flatpak et natif.
- Publie une GitHub Release avec le bundle `.flatpak`, le tarball, le `.deb`, leurs sommes sha256 et le changelog extrait de `CHANGELOG.md`.

### Divers

- `dependabot.yml` : mises à jour hebdomadaires `cargo` et `github-actions`.
- Actions épinglées (tag majeur au minimum, SHA idéalement).
- `GITHUB_TOKEN` minimal : `contents: read` partout, `contents: write` seulement dans `release.yml`.

## Documentation

### README.md (en anglais, pour un public non technique)

1. **Présentation** : une phrase, capture d'écran, badges CI et licence.
2. **Fonctionnalités** : icône d'état, quota, navigateur unifié, dossier de montage par cloud, limite de cache, pause, exclusions, mode headless, hors ligne.
3. **Fournisseurs** : tableau fournisseur / statut (stable, expérimental, prévu) / capacités (quota, lien de partage, corbeille).
4. **Installation sur le bureau** : bundle Flatpak de la release (`flatpak install --user nuage.flatpak`), pré-requis `fuse3`.
5. **Installation sur un serveur (headless)** : `.deb` ou tarball, service systemd, `enable-linger`, connexion d'un compte par tunnel SSH ou par token, exemple complet de bout en bout.
6. **Compiler depuis les sources** : `flatpak-builder`, et développement hors Flatpak (`cargo run`, paquets apt).
7. **Premier lancement** : ajout d'un compte, choix du dossier, limite de cache.
8. **Créer son client OAuth Google** (obligatoire pour Google Drive) pas à pas, avec captures : projet GCP, API Drive, écran de consentement en *Production*, identifiants « Desktop app ».
9. **Permissions Flatpak** : chaque `finish-arg` et sa raison, en particulier `org.freedesktop.Flatpak` (montage FUSE) et `home`.
10. **Offline mode** : épinglage, ce qui marche sans réseau, conflits, espace utilisé.
11. **Dépannage** : icône absente (extension AppIndicator), montage qui échoue (`fuse3`, montage orphelin), dossier hors du home (`flatpak override`), token expiré, démon injoignable.
12. **Architecture** : schéma court, liens vers `docs/ARCHITECTURE.md`, `docs/CLI.md` et `docs/ADDING_A_PROVIDER.md`.
13. **Contributing** et **License** (MIT ; rclone est embarqué sous sa propre licence MIT).

### Autres documents

- `docs/ARCHITECTURE.md` : processus, crates, règles de dépendance, protocole IPC, cycle de vie du démon et de rclone.
- `docs/CLI.md` : toutes les commandes, options, format JSON de chaque sortie, codes de retour.
- `docs/ADDING_A_PROVIDER.md` : module dans `providers`, feature Cargo, enregistrement, icône, tests, ligne dans le tableau du README.
- `CHANGELOG.md` : utilisé par `release.yml`.

## Jalons

| # | Jalon | Livrable |
|---|---|---|
| 1 | **Preuve du point dur** | Flatpak minimal (rclone + wrapper) qui monte un remote Drive configuré à la main dans un dossier du home, visible dans Nautilus. Vérifier aussi les points de [À vérifier](#à-vérifier) marqués « jalon 1 ». **Si ça échoue, on s'arrête et on réévalue.** **Validé le 2026-10-31** : Google Drive monté depuis le Flatpak avec un client OAuth perso, via le relais côté hôte (voir [point dur](#le-point-dur--fuse-dans-le-sandbox-flatpak)) ; navigation, lecture et raccourcis `link.html` vérifiés. |
| 2 | **Squelette** | Workspace Cargo complet (dont `ipc`, `daemon`, `cli`), crates avec leurs interfaces, fournisseur `local`, `ci.yml` vert, README initial, `LICENSE`. |
| 3 | **Démon et CLI** | `RcdSupervisor`, client rc typé, `MountManager`, `UploadControl`, `StatusPoller`, protocole IPC, CLI (`status`, `account`, `mount`, `pause`, `config`), tests de bout en bout avec `local`. |
| 4 | **`nuage-fs` : relais** | Système de fichiers superposé en simple relais vers le montage rclone caché, sans régression ; `MountManager` monte les deux couches ; `nuage-fuse-relay` remplace le relais Python du prototype ; suite de tests FS. |
| 5 | **Google Drive** | Fournisseur `gdrive` (client perso obligatoire, raccourcis web `link.html`), `nuage account add gdrive` avec les deux méthodes OAuth headless, config chiffrée ou `0600`. |
| 6 | **Paquets natifs** | Unit systemd, tarball, `.deb`, job CI `native`. **Nuage est utilisable en headless à partir d'ici.** |
| 7 | **Exclusions** | Filtres rclone sur le montage caché + interception locale dans `nuage-fs` (dossier `local-only`), remontage du seul montage caché, CLI `nuage exclude`. |
| 8 | **Application GTK et tray** | `nuage-gtk` : mise en route guidée (écran client Google compris), fenêtre principale, icône d'état agrégé et menu, démarrage auto (portail Background). |
| 9 | **Navigateur** | Navigateur graphique (navigation, actions, badges, glisser-déposer) et opérations fichiers en CLI. |
| 10 | **Réglages et intégration** | Réglages, comportement de « Quit », notifications, dossier de montage par compte, exclusions dans la GUI. |
| 11 | **Finitions → v1** | Job CI `flatpak`, `release.yml`, AppStream, icône, README et `docs/CLI.md` complets. |
| 12 | **v1.1 — Hors ligne** | Épinglage dans `nuage-fs` : métadonnées SQLite, magasin d'épingles, renvoi, conflits, démarrage hors ligne, GUI + CLI. |
| 13 | **v2** | OneDrive et autres fournisseurs, Drive partagés, puis Flathub. |

## Critères d'acceptation

### v1

**CI et release**

- Sur une PR, la CI est verte : fmt, clippy, tests (dont intégration avec un vrai rclone et le fournisseur `local`), tests de bout en bout de la CLI, cargo-deny, règles de dépendance, builds Flatpak et natif avec artefacts.
- Un tag `v0.1.0` produit une GitHub Release avec le bundle `.flatpak`, le tarball, le `.deb` et leurs sommes sha256.

**Bureau : mise en route**

- Sur une Ubuntu fraîche avec flatpak : installation du bundle, puis ajout du compte Drive entièrement depuis l'interface, auth dans le navigateur, **en 4 écrans** (fournisseur, client Google guidé, authentification, dossier + démarrage auto). Un utilisateur qui suit l'écran guidé crée son client GCP sans consulter d'autre documentation. Un second compte Google ne demande que 3 écrans.
- Aucune autre question n'est posée pendant la mise en route.
- Après « Start sync », la fenêtre se ferme, l'icône est dans la barre système et le dossier est monté.
- Avec le démarrage auto activé, l'icône apparaît au login suivant sans action manuelle.

**Headless**

- Sur un serveur Ubuntu fraîchement installé, sans environnement graphique, accessible en SSH : installation du `.deb` ou du tarball, connexion d'un compte Drive (par tunnel SSH ou par `--token-file`), montage et `nuage status` fonctionnels, **sans aucune invite interactive** lorsque toutes les options sont fournies.
- Un agent (ex. Claude Code) réalise ce scénario de bout en bout en suivant uniquement le README.
- Avec `enable-linger`, le dossier est remonté après un redémarrage du serveur sans connexion d'un utilisateur.
- Chaque commande accepte `--json` et renvoie les codes de retour documentés.

**Montage, sync et cache**

- Le dossier choisi apparaît dans Nautilus et les fichiers s'ouvrent dans les applications de l'hôte. Changer de dossier démonte l'ancien et monte le nouveau.
- Copier un gros fichier dans le dossier monté : l'icône passe à ↻ avec le nombre de fichiers, le fichier porte le badge « Uploading » dans le navigateur, puis tout revient à ✓ (`nuage status` suit les mêmes états).
- En pause, un fichier copié dans le dossier n'est pas envoyé ; il part à la reprise. La pause survit au redémarrage du démon.
- Un Google Doc apparaît dans le dossier monté comme un raccourci qui s'ouvre dans le navigateur web.
- Le quota affiché correspond à celui de drive.google.com (à l'arrondi près).
- Avec une limite de cache de 1 Go, le cache du compte ne dépasse pas durablement 1 Go après lecture de plusieurs Go.

**Navigateur et modularité**

- Le navigateur a le même design pour `gdrive` et `local` ; une action non supportée est masquée, pas en erreur.
- Ajouter un fournisseur déclaratif (ex. OneDrive) ne modifie aucun fichier de `engine`, `ui`, `tray` ni `cli`.

**Exclusions**

- Exclure un dossier depuis le navigateur ou la CLI : après remontage, il disparaît du dossier monté et n'est plus téléchargé ; il reste visible et grisé « Not synced » dans le navigateur et peut être réinclus.
- Un fichier `*.tmp` n'est jamais envoyé.

**Robustesse et sécurité**

- Réseau coupé : état explicite, pas de crash ; retour à la normale ensuite.
- « Quit » avec des uploads en attente (réglage par défaut) : l'interface disparaît, les uploads se terminent, puis le dossier est démonté.
- Après un `kill -9` du démon, le lancement suivant nettoie le montage orphelin.
- Supprimer un compte efface ses données locales (remote, cache, dossier vide) et ne touche à rien sur le cloud.
- Aucun secret dans les logs. Sur le bureau, aucun secret en clair sur le disque ; en headless sans trousseau, `rclone.conf` est en `0600` et `nuage status` le signale.

### v1.1 (hors ligne)

- Un fichier épinglé s'ouvre **à son chemin habituel** dans le dossier monté, réseau coupé.
- Après un **redémarrage sans réseau**, le dossier monté apparaît, l'arborescence est navigable et les fichiers épinglés s'ouvrent.
- Un fichier épinglé modifié hors ligne est renvoyé automatiquement au retour du réseau.
- Une modification locale et distante simultanée produit deux fichiers (aucune perte) et une notification.
- Un nouveau fichier ajouté à distance dans un dossier épinglé est téléchargé automatiquement.
- Remplir le cache VFS au-delà de sa limite n'évince jamais un fichier épinglé.
- Désépingler un fichier dont les modifications n'ont pas été renvoyées est refusé (ou demande confirmation) tant qu'elles ne sont pas envoyées.
- Hors ligne, les éléments non épinglés sont visibles, grisés, et renvoient une erreur propre à l'ouverture, sans bloquer Nautilus.
- Tout cela est aussi pilotable en CLI (`nuage pin …`).
- La CI couvre ces cas avec le fournisseur `local`, en simulant la coupure réseau (arrêt du montage rclone sous-jacent).

## À vérifier

| Point | Quand |
|---|---|
| Le portail FileChooser renvoie bien le vrai chemin (pas `/run/user/*/doc/`) pour un dossier sous le home avec `--filesystem=home` | jalon 1 |
| Noms exacts des champs et options rc (`vfs/stats`, `vfsOpt`, etc.) sur la version embarquée | jalon 3, puis à chaque montée de version (test d'intégration) |
| Le flux « URL + tunnel SSH » avec `config_auth_no_browser` fonctionne de bout en bout avec un vrai compte Google | jalon 5 |
| Build statique (musl) des binaires natifs avec rclone et FUSE | jalon 6 |
| Ouverture des raccourcis `link.html` : l'application HTML par défaut peut ne pas être un navigateur (Thunderbird sur le poste de test). Faut-il que Nuage ouvre lui-même l'URL dans le navigateur par défaut ? | jalon 5 |
| `nuage-fuse-relay` exécuté sur l'hôte depuis le chemin de l'application (`/.flatpak-info`), installation `--user` et système | jalon 4 |
| Le relais reste nécessaire ou non sur d'autres distributions (Fedora, Debian, Arch) ; le garder dans tous les cas s'il est sans effet de bord | jalon 4 |
| Performance de `nuage-fs` en relais (débit, latence des listings) face à un montage rclone direct | jalon 4 |
| rclone n'a pas ajouté d'épinglage natif entre-temps (sinon, réévaluer l'épinglage dans `nuage-fs`) | avant le jalon 12 |

## Questions ouvertes

- **aarch64** (Raspberry Pi, serveurs ARM) : non demandé pour la v1 ; à ajouter aux paquets natifs si le coût est faible.
