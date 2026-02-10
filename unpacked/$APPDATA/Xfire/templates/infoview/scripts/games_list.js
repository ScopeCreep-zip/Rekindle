	var gameslist = { %pac_installed_games% };
	document.write( "<div id='games_list'>\n" );
	for(var p in gameslist)
	{
		document.write( "<div class='games_list_item'><img src='%media_icons_folder%" + p + ".gif' width='16' height='16'><span class='fakelink' action='view_games_page' short_name='" + p + "'>" + gameslist[p] + "</span></div>\n" );
	}
	document.write( "</div>\n" );
