# KivaDB v1.0.1 🚀

KivaDB est un moteur de stockage clé-valeur (Key-Value Store) haute performance et léger, écrit en C. Il utilise une architecture **Log-Structured (Append-Only)** couplée à un **Index Hash Map en mémoire** pour un accès ultra-rapide.

## 🌟 Nouveautés de la v1.0.1
- **Sécurisation du SET** : Empêche l'écrasement accidentel d'une clé existante.
- **Commande UPDATE** : Ajout d'une commande dédiée pour la modification volontaire.
- **Gestion des Données** : Création automatique d'un dossier `data/` pour isoler les fichiers de stockage.
- **Robustesse CLI** : Validation stricte du nombre d'arguments pour éviter les erreurs silencieuses.

## 🚀 Fonctionnalités Clés
- **Stockage Append-Only** : Performance d'écriture maximale via I/O séquentielles.
- **Recherche Rapide** : Indexation par table de hachage en RAM ($O(1)$).
- **Maintenance** : Outils de compactage et statistiques de santé intégrés.
- **Sécurité** : Verrouillage de fichier (File Locking) contre les accès concurrents.

## 📁 Structure du Projet
```text
├── kivadb.exe       # Exécutable principal (Shell)
├── stress_test.exe  # Outil de benchmark
├── data/            # Dossier de stockage (Auto-généré)
│   └── store.kiva   # Base de données persistante
├── src/             # Code source (Core & CLI)
└── include/         # En-têtes API
```

## 🛠️ Installation et Compilation
Recommandé : `gcc` (MinGW ou w64devkit sous Windows).
1. **Cloner le dépôt**:
  ```bash
  git clone https://github.com/fomadev/KivaDB.git
  cd KivaDB
  ```

2. **Compiler**:
  ```bash
  make
  ```

<h2 id="commandes-du-shell">🎮 Commandes du Shell</h2>
<p>Le shell interactif de KivaDB est conçu pour être intuitif et robuste. Voici le guide complet des commandes :</p>

<table style="width:100%; border-collapse: collapse; margin: 25px 0; font-size: 0.9em; font-family: sans-serif; min-width: 400px; box-shadow: 0 0 20px rgba(0, 0, 0, 0.15);">
    <thead>
        <tr style="background-color: #009879; color: #ffffff; text-align: left;">
            <th style="padding: 12px 15px;">Commande</th>
            <th style="padding: 12px 15px;">Syntaxe</th>
            <th style="padding: 12px 15px;">Action</th>
            <th style="padding: 12px 15px;">Exemple</th>
        </tr>
    </thead>
    <tbody>
        <tr style="border-bottom: 1px solid #dddddd;">
            <td style="padding: 12px 15px;"><b>SET</b></td>
            <td style="padding: 12px 15px;"><code>set &lt;clé&gt; &lt;val&gt;</code></td>
            <td style="padding: 12px 15px;">Crée une <b>nouvelle</b> entrée. Échoue si la clé existe déjà.</td>
            <td style="padding: 12px 15px;"><code>set user_1 admin</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd; background-color: #f3f3f3;">
            <td style="padding: 12px 15px;"><b>UPDATE</b></td>
            <td style="padding: 12px 15px;"><code>update &lt;clé&gt; &lt;val&gt;</code></td>
            <td style="padding: 12px 15px;">Modifie une clé <b>existante</b>. Échoue si la clé est absente.</td>
            <td style="padding: 12px 15px;"><code>update user_1 guest</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd;">
            <td style="padding: 12px 15px;"><b>GET</b></td>
            <td style="padding: 12px 15px;"><code>get &lt;clé&gt;</code></td>
            <td style="padding: 12px 15px;">Récupère la valeur associée à une clé.</td>
            <td style="padding: 12px 15px;"><code>get user_1</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd; background-color: #f3f3f3;">
            <td style="padding: 12px 15px;"><b>DEL</b></td>
            <td style="padding: 12px 15px;"><code>del &lt;clé&gt;</code></td>
            <td style="padding: 12px 15px;">Supprime définitivement une clé de la base.</td>
            <td style="padding: 12px 15px;"><code>del session_id</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd;">
            <td style="padding: 12px 15px;"><b>SCAN</b></td>
            <td style="padding: 12px 15px;"><code>scan</code></td>
            <td style="padding: 12px 15px;">Liste toutes les clés actuellement présentes en RAM.</td>
            <td style="padding: 12px 15px;"><code>scan</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd; background-color: #f3f3f3;">
            <td style="padding: 12px 15px;"><b>STATS</b></td>
            <td style="padding: 12px 15px;"><code>stats</code></td>
            <td style="padding: 12px 15px;">Affiche la santé du moteur et la taille occupée sur disque.</td>
            <td style="padding: 12px 15px;"><code>stats</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd;">
            <td style="padding: 12px 15px;"><b>COMPACT</b></td>
            <td style="padding: 12px 15px;"><code>compact</code></td>
            <td style="padding: 12px 15px;">Nettoie le fichier de stockage (défragmentation).</td>
            <td style="padding: 12px 15px;"><code>compact</code></td>
        </tr>
        <tr style="border-bottom: 1px solid #dddddd; background-color: #f3f3f3;">
            <td style="padding: 12px 15px;"><b>HELP</b></td>
            <td style="padding: 12px 15px;"><code>help</code> , <code>h</code> ou <code>\h</code></td>
            <td style="padding: 12px 15px;">Affiche le menu d'aide détaillé.</td>
            <td style="padding: 12px 15px;"><code>\h</code></td>
        </tr>
    </tbody>
</table>
## 📊 Performances
Tests effectués sur SSD :

  * Écriture : ~38 000+ ops/sec (Buffered I/O).

  * Lecture : Quasi-instantanée (Index RAM).

📄 Licence
Ce projet est sous licence <a href="LICENSE">MIT</a>.