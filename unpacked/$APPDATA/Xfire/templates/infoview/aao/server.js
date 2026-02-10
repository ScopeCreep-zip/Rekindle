// custom overrides for America's Army

render_server_extra = function()
{
	var tbody = document.getElementById("server_tbody_id");
	if (tbody)
	{
		var raw_server_info = { %raw_serverinfo% };
		var new_tr = null;
		var new_th = null;
		var new_td = null;

		// PUNKBUSTER				
		if (raw_server_info['sv_punkbuster'] == "1")
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "%js:text_punkbuster%";
			new_td = document.createElement("TD");
			new_td.innerHTML = "<img src='%media_template_folder%infoview/images/punkbuster.gif' alt='Punkbuster Enabled' />";
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}	

		// OFFICIAL SERVER
		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Official";
		new_td = document.createElement("TD");
		if (raw_server_info['official'] == "1")
			new_td.innerText = "Yes";
		else
			new_td.innerText = "No";
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);
				
		// MINHONOR
		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Minimum Honor";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['minhonor'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		// MAXHONOR
		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Maximum Honor";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['maxhonor'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		// GAMEMODE
		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Game Mode";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['gamemode'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		// TOUR
		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Tour";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['tour'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

	}
	
}
