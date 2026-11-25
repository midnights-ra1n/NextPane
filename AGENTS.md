NextPane sera un panel "Vercel-like" où on va pouvoir gérer facilement nos hôtes aved des sites Node.js de déployés. Pour que cela fonctionne, on va devoir installer sur la machine cible un petit agent en Rust (qui devra rester très léger en resources).

Je veux un controle complet de chaque sites, hotes. Je dois pouvoir gérer les ressources de mes applications (les limiter), avoir un monitoring sur ces ressources occupé en faisant la différence de ce qui est occupé par l'hote et les applications (de manière individuelle), taille sur le disque des apps...

On devra pouvoir :
- ajouter des variables d'environnement, les éditer et les exporter (attention l'export en clair est dangereux donc à sécuriser)
- ajouter des clés ssh
- Lié une app à un repo Git (public ou privé) et de n'importe quel provider Git (GitHub, GitLab, Forgejo, Gitea...) et créer des déploiements automatique si il y eu un nouveau commit
- Gestion du cache (par exemple pour une app web si on doit vider le cache etc...)

Il faut que ce soit une sorte de Vercel mais auto-hébergé et en plus light et sans toutes les fonctionnalités que propose Vercel, il ne faut pas non plus que ce soit un coolify. 


# Agent

L'agent devra se connecter via WebSocket et pas forcément en HTTP ou HTTPS, le but n'est pas aussi de monopoliser toute la bande passante. Il devra monitorer tout sur l'hote, pour des raisons de sécurité on devra créer un utilisateur dédié à l'hote qui se supprimera (en conservant les données) ou ne se supprimera pas.

# Interface

Il me faut une interface sombre et complètement noire avec la police Geist et Geist Mono, un peu comme Vercel le fait aussi. Côté logo je veux un petit logo assez sympa inspiré de celui de Next.js mais pour notre projet.


# Déploiement

Je veux que le serveur puisse fonctionner sur Docker via image, Debian/Ubuntu (*.deb), RHEL/Alma/Rocky (*.rpm). De même pour l'agent où il devra fonctionner sur Debian, RHEL et autres (pas de Docker). En ce qui est des méthodes je voudrais aussi un agent qui puisse se déployer via Ansible, en masse ou non. J'aimerais que tu documente un peu tout ça pour l'intégration avec ansible.
