////////////////////////////////////////////
// custom overrides for the matrix online //
////////////////////////////////////////////

render_ip = function()
{
	var game_server_ip = "%js:game_serverip%";
	var strInstance = "";
		
	if (game_server_ip.indexOf("199.108.0.72") != -1 ||
	    game_server_ip.indexOf("199.108.0.73") != -1)
	{
		strInstance = "Recursion";
	}
	
	if (game_server_ip.indexOf("199.108.0.80") != -1 ||
	    game_server_ip.indexOf("199.108.0.81") != -1)
	{
		strInstance = "Vector-Hostile";
	}
	
	if (game_server_ip.indexOf("199.108.0.76") != -1 ||
	    game_server_ip.indexOf("199.108.0.77") != -1)
	{
		strInstance = "Syntax";
	}

	if (strInstance != "")
	{
		// Append new row above IP address
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
					if (tr_element && tr_element.id == "server_ip_row")
					{
						nInsertionRow = rowit;
						break;
					}
				}
			}
			
			var new_tr = tbody.insertRow(nInsertionRow);
			var new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Instance";
			var new_td = document.createElement("TD");
			new_td.innerText = strInstance;
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
	}	
}
