		/////////////////////////////////////////////////////////////////////
		// server.js
		// - Javascript functions related to server templates.
		/////////////////////////////////////////////////////////////////////
		
		var render_server_box = function()
		{
			if (!%user_ingame%)
			{
				show_element("user_box", false);
				document.getElementById("game_server_box_title").innerText = "%js:text_game_server%";
			}

            if (!%is_installed_game%)
                show_element("launch_game_col", false);
                
			if (!%game_haspassword%)
				document.getElementById("game_lock").style.display = "none";
			
			if (!%game_hasquerystatus%)
				show_element("server_query_status_row", false);
				
			if (!%game_hasservername%)
				show_element("server_name_row", false);

			if (!%game_hasip%)
			{
				show_element("server_ip_row", false);
			}
			
			if (!%game_hasserverstatustype% || !%game_hasip%)
			{
				show_element("refresh_server_id", false);
			}
			
			var strServerFlagUrl = "%js:server_flag_url%";
			var bHasServerFlag = strServerFlagUrl.length > 0;
			if (!bHasServerFlag)
				show_element("server_flag", false);
							
			if (!%game_hasserverping%)
				show_element("server_ping_row", false);
				
			if (!%game_hasserverinfo%)
				show_element("server_info_row", false);

			if (!%game_hasservergametype%)
				show_element("server_game_type_row", false);

			if (!%can_launch_rcon%)
				show_element("rcon_row", false);
			else
				render_rcon();
			
			render_server_name();
			render_ip();
			render_game_stats_hash();
			render_player_team_list();
			render_raw_server_info();
			render_server_extra();
		}
		
		var render_server_name = function()
		{
			var element = document.getElementById("server_name");
			if (element)
				element.innerHTML = colorizeName("%js:game_servername%", "%js:servertype%");
		}

		var render_ip = function()
		{
		}
		
		var render_game_stats_hash = function()
		{
			var game_stats_hash = { %game_stats_hash% };
		
			if (%has_game_stats_hash%)
			{
				var tbody = document.getElementById("server_tbody_id");
				if (tbody)
				{
					var tbody_rows = tbody.rows;
					var nInsertionRow = -1; // appends new rows
					if (tbody_rows)
					{
						// want to insert new rows prior to the rcon row
						for (var rowit = 0; rowit < tbody_rows.length; ++rowit)
						{
							var tr_element = tbody_rows.item(rowit);
							if (tr_element && tr_element.id == "rcon_row")
							{
								nInsertionRow = rowit;
								break;
							}
						}
					}

					//alert("insert at: " + nInsertionRow);
					for (var game_stats_key in game_stats_hash)
					{
						var new_tr = tbody.insertRow(nInsertionRow);
						var new_th = document.createElement("TH");
						new_th.className = "first_char_uppercase";
						new_th.innerText = game_stats_key;
						var new_td = document.createElement("TD");
						new_td.innerText = game_stats_hash[game_stats_key];
						new_tr.appendChild(new_th);
						new_tr.appendChild(new_td);
					}
				}
			}
		}

		var render_rcon = function()
		{
			var element = document.getElementById("rcon_password");
			if (element)
				element.value = "%js:rcon_password%";
		}
				
		var render_player_team_list = function()
		{
			var player_team_list = [ %team_data_struct% ];
			if (!player_team_list.length)
				return;

			show_element("player_team_list_id", true);
			
			var player_list_div = document.getElementById("player_team_list_id");
			if (player_list_div)
			{
				var strInnerHTML = "";
				
				// NOTE:  This structure is sorted by FRAGS.
				for (var nTeamIt = 0; nTeamIt < player_team_list.length; nTeamIt++)
				{
					// Start Box
					strInnerHTML += "<div id='player_list_box' class='player_box'>";
					
					// Box Title
					if (player_team_list.length == 1)
						strInnerHTML += "<div id='player_list_box_title' class='box_title'>%js:text_playerlist%</div>";
					else
						strInnerHTML += "<div id='player_list_box_title' class='box_title'>" + player_team_list[nTeamIt].team_name + "</div>";
					
					// Box Details
					strInnerHTML += "<div class='box_details'>";
					strInnerHTML += "  <table class='player_list_table'>";
					strInnerHTML += "    <tr>";
					strInnerHTML += "      <th class='player_list_col_title_0'>%js:text_name%</th>";
					strInnerHTML += "      <td class='player_list_col_title_1'>%js:text_ping%</td>";
					//strInnerHTML += "      <td class='player_list_col_title_2'>%js:text_frags%</td>";
					strInnerHTML += "      <td class='player_list_col_title_2'>%js:game_score_title%</td>";
					strInnerHTML += "    </tr>";
					
					var nTeamFrags = 0;
					for (var nPlayerIt = 0; nPlayerIt < player_team_list[nTeamIt].players.length; nPlayerIt++)
					{
						nTeamFrags += player_team_list[nTeamIt].players[nPlayerIt].frags;
						
						var strPlayerName = player_team_list[nTeamIt].players[nPlayerIt].name;
						var strPlayerNameColorized = colorizeName(strPlayerName, "%js:servertype%");
						
						var in_game_name = null;
						if (%has_game_stats_hash%)
						{
							var game_stats_hash = { %game_stats_hash% };
							if (game_stats_hash['name'])
								in_game_name = game_stats_hash['name'];
						}
						
						strInnerHTML += "    <tr>";

						// If its the selected user then highlight...
						if (in_game_name && in_game_name == strPlayerName)
						{
							strInnerHTML += "      <th class='player_list_col_0_highlighted'>" + strPlayerNameColorized + "</th>";
							strInnerHTML += "      <td class='player_list_col_1_highlighted'>" + player_team_list[nTeamIt].players[nPlayerIt].ping + "</td>";
							strInnerHTML += "      <td class='player_list_col_2_highlighted'>" + player_team_list[nTeamIt].players[nPlayerIt].frags + "</td>";
						}
						else
						{
							strInnerHTML += "      <th class='player_list_col_0'>" + strPlayerNameColorized + "</th>";
							strInnerHTML += "      <td class='player_list_col_1'>" + player_team_list[nTeamIt].players[nPlayerIt].ping + "</td>";
							strInnerHTML += "      <td class='player_list_col_2'>" + player_team_list[nTeamIt].players[nPlayerIt].frags + "</td>";
						}
						
						strInnerHTML += "    </tr>";
					}

					if (player_team_list.length > 1)
					{
						strInnerHTML += "    <tr>";
						strInnerHTML += "      <th class='total_frags' colspan='2'>%js:text_totalfrags%</th>";
						strInnerHTML += "      <td class='total_frags'>" + nTeamFrags + "</td>";
						strInnerHTML += "    </tr>";
					}
					
					strInnerHTML += "  </table>";
					strInnerHTML += "</div>";
					
					// End Box
					strInnerHTML += "</div>";
				}

				player_list_div.innerHTML = strInnerHTML;
			}
		}
		
		var	bShowRawServerInfo = false;
		function ToggleRawServerInfo(link_element)
		{
			var raw_element = document.getElementById("raw_serverinfo_id");
			if (raw_element)
			{
				if (bShowRawServerInfo == false)
				{
					raw_element.style.display = 'block';
					bShowRawServerInfo = true;
					link_element.innerHTML = "%text_hide_rawserverinfo%";
				}
				else
				{
					raw_element.style.display = 'none';
					bShowRawServerInfo = false;
					link_element.innerHTML = "%text_display_rawserverinfo%";
				}
			}
		}
	
		var render_raw_server_info = function()
		{
			var raw_element = document.getElementById("raw_server_info_id");
			if (raw_element)
			{
				var raw_server_info = { %raw_serverinfo% };
				var strDisplayRawServerInfo = "%text_display_rawserverinfo%";
				var strDisplayStyle = "none";
				
				// raw_serverinfo is an associative array and hence .length doesn't work.
				var nLen = 0;
				for (mykey in raw_server_info)
				{
					nLen++;
					break;
				}
				if (nLen > 0)
				{
					strDisplayStyle = "block";
        			show_element("raw_server_info_id", true);
				}

				var strInnerHTML = "";
									
				strInnerHTML += "<span id='raw_serverinfo_link_id' class='fakelink' style='display: " + strDisplayStyle + "' onClick='ToggleRawServerInfo(this);'>" + strDisplayRawServerInfo + "</span>";
				strInnerHTML += "<div id='raw_serverinfo_id' style='display: none'>";
				for (key in raw_server_info)
				{
					var strKey = key;
					var strValue = raw_server_info[key];
					strInnerHTML += "<div>" + strKey + " = " + strValue + "</div>";
				}
				strInnerHTML += "</div>";
				
				raw_element.innerHTML = strInnerHTML;
			}
		}

		var render_server_extra = function()
		{
			// Can be overriden to render extra server info.
		}
		
		/* ATTEMPT TO INCLUDE GAME SPECIFIC OVERRIDE */
		%include server.js%
