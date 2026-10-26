"""MkDocs hook: add Open Graph and Twitter link-preview tags to every page.

LinkedIn, Slack, Teams and X read these tags to build the card shown when a
docs link is shared. The shared rascommander.info theme replaces
docs/overrides/main.html at publish time, so the tags are added here instead
of in a template. The image is docs/images/logo/ras-commander-fast-hdf_social.png
(1200 x 627, the size LinkedIn recommends); regenerate it with
scripts/generate_logos.py and scripts/render_logo_png.ps1.
"""
from html import escape
from urllib.parse import urljoin

SOCIAL_IMAGE = "images/logo/ras-commander-fast-hdf_social.png"
SOCIAL_IMAGE_SIZE = (1200, 627)
SOCIAL_IMAGE_ALT = "RAS Commander Fast-HDF: Native Rust HDF Engine"


def on_post_page(output, page, config, **kwargs):
    site_url = config.get("site_url")
    if not site_url or 'property="og:image"' in output:
        return output

    site_name = config["site_name"]
    title = site_name if page.is_homepage else f"{page.title} - {site_name}"
    description = page.meta.get("description") or config.get("site_description") or ""
    image = urljoin(site_url, SOCIAL_IMAGE)
    url = page.canonical_url or site_url

    tags = {
        "og:type": "website",
        "og:site_name": site_name,
        "og:title": title,
        "og:description": description,
        "og:url": url,
        "og:image": image,
        "og:image:secure_url": image,
        "og:image:type": "image/png",
        "og:image:width": str(SOCIAL_IMAGE_SIZE[0]),
        "og:image:height": str(SOCIAL_IMAGE_SIZE[1]),
        "og:image:alt": SOCIAL_IMAGE_ALT,
    }
    twitter = {
        "twitter:card": "summary_large_image",
        "twitter:title": title,
        "twitter:description": description,
        "twitter:image": image,
        "twitter:image:alt": SOCIAL_IMAGE_ALT,
    }
    lines = [f'<meta property="{k}" content="{escape(v)}">' for k, v in tags.items()]
    lines += [f'<meta name="{k}" content="{escape(v)}">' for k, v in twitter.items()]
    return output.replace("</head>", "\n".join(lines) + "\n</head>", 1)
