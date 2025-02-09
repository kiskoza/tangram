use crate::footer::Footer;
use pinwheel::prelude::*;
use tangram_ui as ui;
use tangram_www_ui::logo::{Logo, LogoColorScheme};

#[derive(children, Default, new)]
#[new(default)]
pub struct Layout {
	pub children: Vec<Node>,
}

impl Component for Layout {
	fn into_node(self) -> Node {
		div()
			.class("layout")
			.child(header().child(Topbar))
			.child(main().child(self.children))
			.child(footer().child(Footer))
			.into_node()
	}
}

struct Topbar;

impl Component for Topbar {
	fn into_node(self) -> Node {
		let gh_button = div().style("line-height", "0").child(
			a().href("https://github.com/tangramdotdev/tangram")
				.class("github-button")
				.class("link")
				.style("color", "var(--text-color)")
				.attribute(
					"data-color-scheme",
					"no-preference: light; light: light; dark: dark;",
				)
				.attribute("data-size", "large")
				.attribute("data-show-count", "true")
				.attribute("aria-label", "Star tangramdotdev/tangram on GitHub")
				.child("GitHub"),
		);
		let topbar_items = vec![
			ui::TopbarItem {
				element: Some(gh_button.into_node()),
				href: "https://github.com/tangramdotdev/tangram".to_owned(),
				title: "GitHub".to_owned(),
			},
			ui::TopbarItem {
				element: None,
				href: "/jobs".to_owned(),
				title: "Jobs".to_owned(),
			},
			ui::TopbarItem {
				element: None,
				href: "/blog/".to_owned(),
				title: "Blog".to_owned(),
			},
			ui::TopbarItem {
				element: None,
				href: "/pricing".to_owned(),
				title: "Pricing".to_owned(),
			},
			ui::TopbarItem {
				element: None,
				href: "/docs/".to_owned(),
				title: "Docs".to_owned(),
			},
			ui::TopbarItem {
				element: None,
				href: "/benchmarks".to_owned(),
				title: "Benchmarks".to_owned(),
			},
			ui::TopbarItem {
				element: None,
				href: "https://app.tangram.dev".to_owned(),
				title: "Login".to_owned(),
			},
			ui::TopbarItem {
				element: Some(
					ui::Button::new()
						.color(ui::colors::BLUE.to_owned())
						.href("/docs/getting_started/".to_owned())
						.child("Learn")
						.into(),
				),
				href: "/docs/getting_started/".to_owned(),
				title: "Learn".to_owned(),
			},
		];
		ui::Topbar::new()
			.background_color(ui::colors::HEADER.to_owned())
			.dropdown_background_color(ui::colors::SURFACE.to_owned())
			.items(topbar_items)
			.logo(Logo::new().color_scheme(LogoColorScheme::Multi).into_node())
			.title("tangram".to_owned())
			.into_node()
	}
}
