		/////////////////////////////////////////////////////////////////////
		// user.js
		// - Javascript functions related to user and server templates.
		/////////////////////////////////////////////////////////////////////
		
		var render_user_box = function()
		{
		    if (!%enable_user_avatar%)
		        show_element("user_avatar_id", false);
		        
		    if (!%is_user_blocked%)
		        show_element("blocked_user_link", false);
		    
			if (!%user_has_clan_nickname% || ("%js:user_display_name%" == "%js:clan_member_nickname%"))
			{
			    show_element("user_clan_nickname_id", false);
				show_element("user_clan_nickname_row", false);
			}
			
			if (!%user_hasnickname% || ("%js:user_display_name%" == "%js:user_nickname%"))
			{
			    show_element("user_nickname_id", false);
				show_element("user_nickname_row", false);
			}
				
			if ("%js:user_display_name%" == "%js:username%")
			{
			    show_element("username_id", false);
				show_element("username_row", false);
			}

			if (!%voice_hasip%)
			{
			    show_element("voice_chat_id", false);
				show_element("voice_chat_row", false);
			}
				
			render_status();
		}

		var render_status = function()
		{
			var element = document.getElementById('status_id');
			if (element)
			{
				element.innerHTML = linkify("%js:status%");
			}
		}
		
		var render_clans_box = function()
		{
			var user_clan_info = [ %user_clan_info% ];
			if (!user_clan_info.length)
				show_element("clans_box", false);

            var element = document.getElementById('clans_box_detail_id');
            if (element)
            {
                var strInnerHTML = "<table class='detail_table'>";
			    for (var x = 0; x < user_clan_info.length; x++)
			    {
			        strInnerHTML += "<tr>";
			        strInnerHTML += "<th><a href='" + user_clan_info[x].clanurl + "' target='_blank'>" + user_clan_info[x].clanname + "</a></th>";
			        strInnerHTML += "<td>" + user_clan_info[x].nickname + "</td>";
			        strInnerHTML += "</tr>";
			    }
			    strInnerHTML += "</table>";
			    element.innerHTML = strInnerHTML;
            }
            
			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
            RebuildEventSinks();
		}

        var render_user_videos = function()
        {
            var video_array = [ %user_videos_data% ];
            if (!video_array.length)
            {
                show_element("user_videos_box", false);
                return;
            }
            
            // Show videos box
            show_element("user_videos_box", true);

            // Insert up to 2 of this users latest VIDEO thumbs
			var user_videos_div = document.getElementById("user_videos_id");
			if (!user_videos_div)
			    return;

            var strInnerHTML = "";
            strInnerHTML += "<table border='0' >";
            strInnerHTML += "<tr>";
            for (var i = 0; (i < video_array.length) && (i < 2); ++i)
            {
                var strThumbnailURL = video_array[i].thumb_url;
                var strThumbnailHREF = video_array[i].video_url;

                strThumbnailURL = strThumbnailURL.replace(
                    /video\.xf1re\.com/,
                    'xf1re.b-cdn.net/videos'
                );
                var created = new Date(video_array[i].timestamp * 1000);

                var strAltText = created.toLocaleString() + "\n" + video_array[i].gamename + "\n";
                strAltText += video_array[i].title + "\n" + video_array[i].description;
                
                strInnerHTML += "<td>";
                strInnerHTML += "<a href='" + strThumbnailHREF + "' target='_blank'><img src='" + strThumbnailURL + "' alt='" + strAltText + "' style='cursor:pointer;border:none;' /></a>";
                strInnerHTML += "</td>";
            }
            strInnerHTML += "</tr>";
            strInnerHTML += "<tr><td colspan='2' align='center'><a href='%js:user_videos_url%' target='_blank'>%js:text_see_all_videos%</a></td></tr>";
            strInnerHTML += "</table>";
            
            user_videos_div.innerHTML = strInnerHTML;
            
			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
            RebuildEventSinks();
        }
        
        var render_user_ss = function()
        {
            // Hide the box until we get a valid response back that this user has remote screenshots.
            show_element("user_ss_box", false);
            
            // Check client setting to see if we are requesting user screenshots
            request_ss_count();
            
			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
            RebuildEventSinks();
        }
        
        function request_ss_count()
        {
            var screenshot_array = [ %user_ss_data% ];
            if (!screenshot_array.length)
                return;
            
            // Show screenshots box
            show_element("user_ss_box", true);

            // Insert up to 2 of this users latest screenshot thumbs
			var user_ss_div = document.getElementById("user_ss_id");
			if (!user_ss_div)
			    return;

            var strInnerHTML = "";
            strInnerHTML += "<table border='0' >";
            strInnerHTML += "<tr>";
            for (var i = 0; (i < screenshot_array.length) && (i < 2) && (screenshot_array[i].filename.length); ++i)
            {
                var strThumbnailURL = "http://%js:screenshot_host%/s/" + screenshot_array[i].remoteid + "-0.jpg";
                var strThumbnailHREF = "%js:www_url%/profile/%js:username%/screenshots/?view#" + screenshot_array[i].remoteid;
                var created = new Date(screenshot_array[i].timestamp * 1000);
                var strAltText = created.toLocaleString() + "\n" + screenshot_array[i].gamename;
                if (screenshot_array[i].description.length)
                    strAltText += "\n" + screenshot_array[i].description;
				strAltText = strAltText.replace(/'/, "&#39;");
                strInnerHTML += "<td>";
                strInnerHTML += "<a href='" + strThumbnailHREF + "' target='_blank'><img src='" + strThumbnailURL + "' alt='" + strAltText + "' style='cursor:pointer;border:none;' /></a>";
                strInnerHTML += "</td>";
            }
            strInnerHTML += "</tr>";
            strInnerHTML += "<tr><td colspan='2' align='center'><a href='%js:www_url%/profile/%js:username%/screenshots/' target='_blank'>%js:text_see_all_ss%</a></td></tr>";
            strInnerHTML += "</table>";
            
            user_ss_div.innerHTML = strInnerHTML;
        }
		
		var render_user_broadcast = function()
        {
            var broadcast_info = { %user_broadcast_data% };
            if (!broadcast_info.streamsid || !broadcast_info.streamsid.length)
            {
                show_element("user_broadcast_box", false);
                return;
            }
                
            // Show videos box
            show_element("user_broadcast_box", true);

            // Insert up to 2 of this users latest VIDEO thumbs
			var user_broadcast_div = document.getElementById("user_broadcast_id");
			if (!user_broadcast_div)
			    return;

            var strInnerHTML = "";
            //strInnerHTML += "<table border='1'>";
            //strInnerHTML += "<tr>";

			var strThumbnailURL = broadcast_info.thumbnail_url;
			var strThumbnailHREF = broadcast_info.broadcast_url;
			var strAltText = broadcast_info.title + "\n" + broadcast_info.description + "\n";

			strInnerHTML += "<table border='0' cellpadding='0' cellspacing='0'><tr><td>";
			strInnerHTML += "<a href='" + strThumbnailHREF + "' target='_blank' elevated='1'><div><img src='" + strThumbnailURL + "' alt='" + strAltText + "' width='132' height='100' style='cursor:pointer;border:none;' /></div>";
			strInnerHTML += "<div style='float:left; position:relative; top:-23px; left:52px;'><img src='%media_template_folder%infoview/images/video_live_preview_overlay.gif' width='80' height='23' alt='" + strAltText + "' style='cursor:pointer;border:none;' /></div></a>";
			strInnerHTML += "</td></tr></table>";
            
            //strInnerHTML += "</tr>";
            //strInnerHTML += "</table>";
            
            user_broadcast_div.innerHTML = strInnerHTML;
            
			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
            RebuildEventSinks();
        }

		// ATTEMPT TO INCLUDE OVERRIDE
		%include user.js%
