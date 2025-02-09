use crate::page::{Owner, Page};
use anyhow::Result;
use pinwheel::prelude::*;
use sqlx::prelude::*;
use std::{borrow::BorrowMut, sync::Arc};
use tangram_app_context::Context;
use tangram_app_core::{
	error::{redirect_to_login, service_unavailable},
	user::{authorize_user, User},
};
use tangram_app_layouts::app_layout::app_layout_info;

pub async fn get(request: &mut http::Request<hyper::Body>) -> Result<http::Response<hyper::Body>> {
	let context = Arc::clone(request.extensions().get::<Arc<Context>>().unwrap());
	let app_state = &context.app.state;
	let mut db = match app_state.database_pool.begin().await {
		Ok(db) => db,
		Err(_) => return Ok(service_unavailable()),
	};
	let user = match authorize_user(request, &mut db, app_state.options.auth_enabled()).await? {
		Ok(user) => user,
		Err(_) => return Ok(redirect_to_login()),
	};
	let app_layout_info = app_layout_info(app_state).await?;
	let owners = match user {
		User::Root => None,
		User::Normal(user) => {
			let mut owners = vec![Owner {
				value: format!("user:{}", user.id),
				title: user.email,
			}];
			let rows = sqlx::query(
				"
				select
					organizations.id,
					organizations.name
				from organizations
				join organizations_users
					on organizations_users.organization_id = organizations.id
					and organizations_users.user_id = $1
			",
			)
			.bind(&user.id.to_string())
			.fetch_all(db.borrow_mut())
			.await?;
			for row in rows {
				let id: String = row.get(0);
				let title: String = row.get(1);
				owners.push(Owner {
					value: format!("organization:{}", id),
					title,
				})
			}
			Some(owners)
		}
	};
	let page = Page {
		app_layout_info,
		owners,
		error: None,
		owner: None,
		title: None,
	};
	let html = html(page);
	let response = http::Response::builder()
		.status(http::StatusCode::OK)
		.body(hyper::Body::from(html))
		.unwrap();
	Ok(response)
}
