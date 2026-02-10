		//////////////////////////////////////////////////////////////////////////////////////////////
		// CreateCheckBox()
		// - Helper function to create the following HTML DOM:
		// <div>
		//   <label for='official'><input id='official' type='checkbox' name='official'>Official</label>
		// </div>
		//////////////////////////////////////////////////////////////////////////////////////////////
		function CreateCheckBox(id, name, text)
		{
			var div_element = document.createElement("DIV");
			var label_element = document.createElement("LABEL");
			label_element.htmlFor = id;
			var input_element = document.createElement("INPUT");
			input_element.id = id;
			input_element.type = "checkbox";
			input_element.name = name;
			var text_element = document.createTextNode(text);
			
			label_element.appendChild(input_element);
			label_element.appendChild(text_element);
			div_element.appendChild(label_element);
			return div_element;
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// CreateRow()
		// - Helper function to create the following HTML DOM:
		// <tr>
		//	 <th>%js:text_min_players%:</th>
		//	 <td><input id='xf_numplayers_min' class='text' type='text' size='15' name='xf_numplayers_min' onkeypress='disableEnterKey()'/></td>
		// </tr>
		//////////////////////////////////////////////////////////////////////////////////////////////
		function CreateRow(id, name, text)
		{
			var tr_element = document.createElement("TR");
			var th_element = document.createElement("TH");
			var text_element = document.createTextNode(text);
			var td_element = document.createElement("TD");
			var input_element = document.createElement("INPUT");
			input_element.id = id;
			input_element.className = "text";
			input_element.type = "text";
			input_element.size = 15;
			input_element.name = name;
			input_element.attachEvent("onkeypress", disableEnterKey);

			th_element.appendChild(text_element);
			td_element.appendChild(input_element);
			tr_element.appendChild(th_element);
			tr_element.appendChild(td_element);
			return tr_element;			
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// RenderGameSpecificBox()
		// - Renders Americas Army Game Specific Filters.
		// - Allows game specific filters to render data in the custom_box.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		RenderGameSpecificBox = function()
		{
			// Make sure the box is visible.
			var box = document.getElementById("game_specific_box");
			if (box)
				box.style.display = "block";
			
			// Checkboxes...
			var parent_element = document.getElementById("game_specific_checkboxes_here");
			if (parent_element)
			{
				var new_checkbox = CreateCheckBox("secure", "secure", "VAC Secured");
				if (new_checkbox)
				{
					parent_element.appendChild(new_checkbox);
				}
					
				new_checkbox = CreateCheckBox("password", "password", "%js:text_no_password%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);
				
				new_checkbox = CreateCheckBox("sv_voiceenable", "sv_voiceenable", "Voice Enabled");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("mp_friendlyfire", "mp_friendlyfire", "Friendly Fire On");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);
					
				new_checkbox = CreateCheckBox("linux", "linux", "Only Linux Servers");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);
					
				new_checkbox = CreateCheckBox("windows", "windows", "Only Windows Servers");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);
			}
			
			// Table rows...
			var parent_element = document.getElementById("game_specific_table_here");
			if (parent_element)
			{
				var new_row = CreateRow("minround", "minround", "Min Round Time");
				if (new_row)
					parent_element.appendChild(new_row);
					
				new_row = CreateRow("maxround", "maxround", "Max Round Time");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("minfreeze", "minfreeze", "Min Freeze Time");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("maxfreeze", "maxfreeze", "Max Freeze Time");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("minc4", "minc4", "Min C4 Timer");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("maxc4", "maxc4", "Max C4 Timer");
				if (new_row)
					parent_element.appendChild(new_row);

			}			
		}

		//////////////////////////////////////////////////////////////////////////////////////////////
		// GetFilters()
		// - Returns the string representation of the filter: i.e. xf_hideempty==1;protocol~~68;
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		GetFilters = function()
		{
			var xf_hideempty = document.getElementById('xf_hideempty');
			var xf_hidefull = document.getElementById('xf_hidefull');
			var xf_servername = document.getElementById('xf_servername');
			var xf_mapname = document.getElementById('xf_mapname');
			var xf_gametype = document.getElementById('xf_gametype');
			var xf_ping = document.getElementById('xf_ping');
			var xf_numplayers_min = document.getElementById('xf_numplayers_min');
			var xf_numplayers_max = document.getElementById('xf_numplayers_max');
			var xf_player = document.getElementById('xf_player');
			var country_combo = document.getElementById('xf_country');
		
			var secure = document.getElementById('secure');
			var sv_voiceenable = document.getElementById('sv_voiceenable');
			var mp_friendlyfire = document.getElementById('mp_friendlyfire');
			var password = document.getElementById('password');
			var linux = document.getElementById('linux');
			var windows = document.getElementById('windows');
			
			var maxc4 = document.getElementById('maxc4');
			var minc4 = document.getElementById('minc4');
			var maxround = document.getElementById('maxround');
			var minround = document.getElementById('minround');
			var maxfreeze = document.getElementById('maxfreeze');
			var minfreeze = document.getElementById('minfreeze');
			
			var str = "";

			if (xf_hideempty && xf_hideempty.checked)
			{
				str += "xf_hideempty==1;";
			}
			if (xf_hidefull && xf_hidefull.checked)
			{
				str += "xf_hidefull==1;";
			}
			if (xf_servername && xf_servername.value != "")
			{
				str += "xf_servername~~" + escapeString(xf_servername.value) + ";";
			}
			if (xf_mapname && xf_mapname.value != "")
			{
				str += "xf_mapname~~" + escapeString(xf_mapname.value) + ";";
			}
			if (xf_gametype && xf_gametype.value != "")
			{
				str += "xf_gametype~~" + escapeString(xf_gametype.value) + ";";
			}
			if (xf_ping && xf_ping.value != "")
			{
				str += "xf_ping<=" + escapeString(xf_ping.value) + ";";
			}
			if (xf_numplayers_min && xf_numplayers_min.value != "")
			{
				str += "xf_numplayers>=" + escapeString(xf_numplayers_min.value) + ";";
			}
			if (xf_numplayers_max && xf_numplayers_max.value != "")
			{
				str += "xf_numplayers<=" + escapeString(xf_numplayers_max.value) + ";";
			}
			if (xf_player && xf_player.value != "")
			{
				str += "xf_player~~" + escapeString(xf_player.value) + ";";
			}
			
			if (password && password.checked)
			{
				str += "password!=1;";
			}
			if (secure && secure.checked)
			{
				str += "secure==1;";
			}
			if (sv_voiceenable && sv_voiceenable.checked)
			{
				str += "sv_voiceenable==1;";
			}
			if (mp_friendlyfire && mp_friendlyfire.checked)
			{
				str += "mp_friendlyfire==1;";
			}
			if (linux && windows)
			{
				if (linux.checked && !windows.checked)
				{
					str += "os==108;";
				}
				if (windows.checked && !linux.checked)
				{
					str += "os==119;";
				}
			}
			
			if (maxc4 && maxc4.value != "")
			{
				str += "mp_c4timer<=" + escapeString(maxc4.value) + ";";
			}
			if (minc4 && minc4.value != "")
			{
				str += "mp_c4timer>=" + escapeString(minc4.value) + ";";
			}
			if (maxround && maxround.value != "")
			{
				str += "mp_roundtime<=" + escapeString(maxround.value) + ";";
			}
			if (minround && minround.value != "")
			{
				str += "mp_roundtime>=" + escapeString(minround.value) + ";";
			}
			if (maxfreeze && maxfreeze.value != "")
			{
				str += "mp_freezetime<=" + escapeString(maxfreeze.value) + ";";
			}
			if (minfreeze && minfreeze.value != "")
			{
				str += "mp_freezetime>=" + escapeString(minfreeze.value) + ";";
			}
			
			if (country_combo)
			{
				var nSelectedIndex = country_combo.selectedIndex;
				var strVal = country_combo.options[nSelectedIndex].value;
				// Only save out if != "all"
				if (strVal != "all")
				{
					str += "xf_country~~" + strVal + ";";
				}
			}
			
			// Advanced filters
			///////////////////
			var table_element = document.getElementById("raw_table");
			if (table_element)
			{
				if (table_element.hasChildNodes() == true)
				{
					var node = table_element.firstChild;
					while (node)
					{
						if (node.nodeName == "TR")
						{
							// Each ROW should have 3 SELECT elements, one for KEY, one for EXPRESSION, one for VALUE.
							var select_elements = node.getElementsByTagName("SELECT");
							if (select_elements && select_elements.length == 3)
							{
								// key is select_element[0]
								var keySelect = select_elements[0];
								var strKey = keySelect.options[keySelect.selectedIndex].value;

								// expression is select_element[1]
								keySelect = select_elements[1];
								var strExpression = keySelect.options[keySelect.selectedIndex].value;
								
								// value is select_element[2]
								keySelect = select_elements[2];
								var strValue = keySelect.options[keySelect.selectedIndex].value;
								
								//alert("key: " + strKey + ", value: " + strValue);
								var strNone = "%js:text_combo_none%";
								if (strKey != strNone)
									str += strKey + strExpression + strValue + ";";
							}
						}
						node = node.nextSibling;
					}
				}
			}
					
			return str;
		}

		//////////////////////////////////////////////////////////////////////////////////////////////
		// ClearAll()
		// - Resets everything on the page.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		ClearAll = function()
		{
			// combo box
			var element = document.getElementById('xf_country');
			if (element)
				element.selectedIndex = 0;

			// checkboxes
			element = document.getElementById('xf_hideempty');
			if (element)
				element.checked = false;
			element = document.getElementById('xf_hidefull');
			if (element)
				element.checked = false;
			element = document.getElementById('secure');
			if (element)
				element.checked = false;
			element = document.getElementById('sv_voiceenable');
			if (element)
				element.checked = false;
			element = document.getElementById('mp_friendlyfire');
			if (element)
				element.checked = false;
			element = document.getElementById('password');
			if (element)
				element.checked = false;
			element = document.getElementById('linux');
			if (element)
				element.checked = false;
			element = document.getElementById('windows');
			if (element)
				element.checked = false;
			
			// text entries
			element = document.getElementById('xf_servername');
			if (element)
				element.value = "";
			element = document.getElementById('xf_mapname');
			if (element)
				element.value = "";
			element = document.getElementById('xf_gametype');
			if (element)
				element.value = "";
			element = document.getElementById('xf_ping');
			if (element)
				element.value = "";
			element = document.getElementById('xf_numplayers_min');
			if (element)
				element.value = "";
			element = document.getElementById('xf_numplayers_max');
			if (element)
				element.value = "";
			element = document.getElementById('xf_player');
			if (element)
				element.value = "";
			element = document.getElementById('maxc4');
			if (element)
				element.value = "";
			element = document.getElementById('minc4');
			if (element)
				element.value = "";
			element = document.getElementById('maxround');
			if (element)
				element.value = "";
			element = document.getElementById('minround');
			if (element)
				element.value = "";
			element = document.getElementById('maxfreeze');
			if (element)
				element.value = "";
			element = document.getElementById('minfreeze');
			if (element)
				element.value = "";
			
			// advanced filters
			var table_element = document.getElementById("raw_table");
			if (table_element)
			{
				// Remove all rows.
				while (table_element.rows.length > 0)
					table_element.deleteRow(0);
			}
			
			// If we don't have any raw server info then inform user to refresh the filter.
			if (associative_array_length(gFilterRawKeyValues) == 0)
			{
				var tr_element = document.createElement("TR");
				var th_element = document.createElement("TD");
				th_element.colSpan = 4;
				var text_element = document.createTextNode("%text_empty_rawserver_keyvalues%");
				th_element.appendChild(text_element);
				tr_element.appendChild(th_element);
				document.getElementById("raw_table").appendChild(tr_element);
			}
			else
			{
				// If we have server info key/values, then we will be wanting an ADD row button.
				// Show the one-and-only ADD row icon
				var tr_element = document.createElement("TR");
				var th_element = document.createElement("TH");
				var span_element = document.createElement("SPAN");
				span_element.id = "add_raw_row_id";
				span_element.className = "fake_href";
				span_element.setAttribute("name", "AddRemoveRow");
				span_element.attachEvent("onclick", OnAddRawRow);
				var img_element = document.createElement("IMG");
				img_element.src = "%media_template_folder%infoview/images/icon_add.gif";
				img_element.title = "%text_add%";
				span_element.appendChild(img_element);
				th_element.appendChild(span_element);
				tr_element.appendChild(th_element);
				tr_element.appendChild(document.createElement("TD"));
				tr_element.appendChild(document.createElement("TD"));
				tr_element.appendChild(document.createElement("TD"));
				document.getElementById("raw_table").appendChild(tr_element);
			}
			
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// SetFilters()
		// - Called on PAGELOADDONE and whenever we want to reset the filter infoview.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		SetFilters = function(filtersstr)
		{
			//alert("SetFilters: " + filtersstr);
			
			// First clear everything out.
			ClearAll();
			
			// Place filter data in appropriate fields.
			var bRawServerInfoAdded = false;
			var filters = splitEscaped(filtersstr);
			for (var i = 0; i < filters.length; i++)
			{
				var filter = parseFilter(filters[i]);
				if (filter != null)
				{
					var strKey = filter[0];
					var strExpression = filter[1];
					var strValue = filter[2];
					
					var element = null;
					if (strKey == "os")
					{
						if (strValue == 108)
						{
							element = document.getElementById('linux');
							if (element)
								element.checked = 1;
						}
						else if (strValue == 119)
						{
							element = document.getElementById('windows');
							if (element)
								element.checked = 1;
						}
						continue;
					}
					
					if (strKey == "mp_c4timer")
					{
						if (strExpression == '>=')
							element = document.getElementById('minc4');
							if (element)
								element.value = strValue;
							
						if (strExpression == '<=')
							element = document.getElementById('maxc4');
							if (element)
								element.value = strValue;
							
						continue;
					}
					
					if (strKey == "mp_roundtime")
					{
						if (strExpression == '>=')
							element = document.getElementById('minround');
							if (element)
								element.value = strValue;
							
						if (strExpression == '<=')
							element = document.getElementById('maxround');
							if (element)
								element.value = strValue;
						
						continue;
					}
					
					if (strKey == "mp_freezetime")
					{
						if (strExpression == '>=')
							element = document.getElementById('minfreeze');
							if (element)
								element.value = strValue;
							
						if (strExpression == '<=')
							element = document.getElementById('maxfreeze');
							if (element)
								element.value = strValue;
							
						continue;
					}
					
					var obj = null;
					if (strKey == "xf_numplayers")
					{
						if (strExpression == "<=")
							obj = document.getElementById(strKey + "_max");
						else if (strExpression == ">=")
							obj = document.getElementById(strKey + "_min");
					}
					else
					{
						obj = document.getElementById(strKey);
					}
					
					if (obj)
					{
						// Must be an HTML element built into the filter template.
						if (obj.type == 'checkbox')
						{
							if (strKey == "password")
							{
								if (strExpression == '!=')
									obj.checked = (strValue != 0);
								else
									obj.checked = (strValue == 0);
							}
							else
							{
								if (strExpression == '==')
									obj.checked = (strValue != 0);
								else
									obj.checked = (strValue == 0);
							}
						}
						else if (obj.type == 'text')
						{
							obj.value = strValue;
						}
						else
						{
							if (strKey == "xf_country")
							{
								for (var j = 0; j < obj.length; j++)
								{
									if (obj.options[j].value == strValue)
									{
										obj.options[j].selected = true;
									}
								}
							}
						}
					}
					else
					{
						// If it's not an HTML element in the filter template, then it must be an
						// advanced raw server key/value filter.  Add NEW items to raw server table.
						//alert("Add raw item: " + strKey + strExpression + strValue);
						AddRawKeyValue(strKey, strExpression, strValue);
						bRawServerInfoAdded = true;
					}
				}
			}

			// What the user sees underneath the Advanced Filters section depends on whether
			// the raw server data is empty and whether or not any raw key values were set.
			if (associative_array_length(gFilterRawKeyValues) != 0)
			{
				// We have raw server data but NO key values were selected, show combo box with <none> selected.
				if (bRawServerInfoAdded == false)
				{
					// Empty will default selection to <none>.
					AddRawKeyValue("", "", "");
				}
			}

			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
			RebuildEventSinks();
		}

