// Global variables
var itemcount = 0;
var itemtotal = 0;
var responseXML = null;

var render_blog_box = function()
{
	if (%is_plugin_user%)
	{
	    show_element("blog_box", false);
	    return;
	}

	show_element("blog_nav_bar", false);
}

var render_blog = function()
{
	show_element("blog_title_bar", true);
	show_element("blog_load_bar", false);
	
	var blog_element = document.getElementById("blog");
	blog_element.innerHTML = "<br><div align='center'>%js:user_loading_blog%</div>";
	request_blog_entries();
}

function request_blog_entries()
{
	AjaxRequest.get(
	{
		'url':'%scripting_host%/blog/%username%/rss/',
		'onSuccess':function(response)
			{
				responseXML = response.responseXML; 
				display_blog(responseXML, 0);
			}
	}
	);
}

function display_blog(data, bitem)
{
	if(data != null)
	{
		var item = data.getElementsByTagName('item').item(bitem);
		if(item != null)
		{
			var date = item.getElementsByTagName('xfire:lastActivity').item(0).firstChild.data;
			if (date != "Unknown")
			{
				//Make the entire thing visible
				show_element("blog_nav_bar", true);
		
				var display = "";
				if(bitem < item.length || bitem >= 0)
				{
					itemtotal = data.getElementsByTagName('item').length - 1;
					var firstInd = document.getElementById("first");
					var prevInd = document.getElementById("prev");
					var lastInd = document.getElementById("last");
					var nextInd = document.getElementById("next");

					if(bitem == 0 && bitem != itemtotal)
					{
						firstInd.style.visibility="hidden";
						prevInd.style.visibility="hidden";
						lastInd.style.visibility="";
						next.style.visibility="";
					}if(bitem == itemtotal && bitem != 0)
					{
						firstInd.style.visibility="";
						prevInd.style.visibility="";
						lastInd.style.visibility="hidden";
						next.style.visibility="hidden";
					}if(bitem > 0)
					{
						firstInd.style.visibility="";
						prevInd.style.visibility="";

					}if(bitem < itemtotal)
					{
						lastInd.style.visibility="";
						next.style.visibility="";
					}
				
					var title = item.getElementsByTagName('title').item(0).firstChild.data;
					var description = item.getElementsByTagName('description').item(0).firstChild.data;
					var num_comments = item.getElementsByTagName('xfire:blogComments').item(0).firstChild.data;
					var num_views = item.getElementsByTagName('xfire:blogViews').item(0).firstChild.data;
					var entry_link = item.getElementsByTagName('guid').item(0).firstChild.data;
			
					// print out entry item in title bar
					var entry_count = "( " +(bitem+1)+ " %js:text_of% " +(itemtotal+1)+ " )";
					document.getElementById("entry_count_id").innerHTML = entry_count;
				
					//Display the blog
					var blog_element = document.getElementById("blog");
					if (blog_element)
					{
						blog_element.innerHTML = "<div class='blog_title'><a href='" + entry_link + "' target='_blank'>" + title + "</a></div>";
						blog_element.innerHTML += "<div class='blog_date'>" + date + "</div><br>";
						blog_element.innerHTML += "<div>" + description + "</div><br>";
						blog_element.innerHTML += "<div>";
						blog_element.innerHTML += "<a href='" + entry_link + "&#35;comments' target='_blank'>" + num_comments + " %js:text_comments%</a> - ";
						blog_element.innerHTML += "<a href='" + entry_link + "reply' target='_blank'>%js:text_post_a_comment%</a> - ";
						blog_element.innerHTML += "%js:text_views%: " + num_views + "</div>";
					}
				    
				}
			}
			else
			{
				var blog_element = document.getElementById("blog");
				var user_blog_link = document.getElementById("user_blog_link");
				
				blog_element.innerHTML = "<br><div align='center'>%user_blocked_blog%</div>";
				user_blog_link.style.color = blog_element.style.color="rgb(%color_text%)";
				user_blog_link.style.textDecoration = document.getElementById("blog").style.textDecoration;
				user_blog_link.outerHTML = "<div>" + user_blog_link.innerHTML + "</div>";
			}
		}
		else
		{
			var blog_element = document.getElementById("blog");
			var user_blog_link = document.getElementById("user_blog_link");
			
			blog_element.innerHTML = "<br><div align='center'>%text_user_has_no_blog%</div>";
			user_blog_link.style.color = blog_element.style.color="rgb(%color_text%)";
			user_blog_link.style.textDecoration = document.getElementById("blog").style.textDecoration;
			user_blog_link.outerHTML = "<div>" + user_blog_link.innerHTML + "</div>";
			
		}
	}
}

function get_blog_post(action)
{
	switch(action)
	{
	case (action="next"):
	    itemcount = itemcount + 1;
	    display_blog(responseXML, itemcount);
	    break;
	case (action="prev"):
	    itemcount = itemcount - 1;
	    display_blog(responseXML, itemcount);
	    break;
	case (action="first"):
	    itemcount = 0;
	    display_blog(responseXML, 0);
	    break;
	case (action="last"):
	    itemcount = itemtotal;
	    display_blog(responseXML, itemtotal);
	    break;
	}
}
