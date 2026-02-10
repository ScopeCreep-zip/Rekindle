	/////////////////////////////////////////////////////////////////
	//                          IMPORTANT!!!                       //
	/////////////////////////////////////////////////////////////////
	// If you're updating this, ensure you update the corresponding
	// php on the website and the C++ version in the client.


	var gameColorTable =
	{
		COD: [ "darkgrey", "red", "#00DC00", "yellow",  "blue",    "cyan",    "magenta", "white", null,  null ],
		Q3A: [ "darkgrey", "red", "#00DC00", "#FFFF14", "#5A96FF", "#00A0A0", "#C332BE", "white", "red", "green" ],
		DOOM3: [ "darkgrey", "red", "#00DC00", "yellow", "blue", "cyan", "magenta", "white", "darkgrey", "black" ],
		FARCRY: ["black", "white", "blue", "green", "red", "cyan", "yellow", "pink", "orange", "darkgrey" ],
		WOLFET: ["black", "red",  "lime",  "yellow", "blue",  "cyan",  "magenta", "white", 
		"#FF8000", "gray",  "silver", "silver", "green", "olive", "navy",  "maroon",
		"#804000", "#FF991A", "teal",  "purple", "#0080FF", "#8000FF", "#3399CC", "#CCFFCC",
		"#006633", "#FF0033", "#B31A1A", "#993300", "#CC9933", "#999933", "#FFFFBF", "#FFFF80" ]

	};
	
	// same color tables & different types
	gameColorTable["MOHAA"] = gameColorTable["Q3A"];
	gameColorTable["Q4"] = gameColorTable["DOOM3"];
	gameColorTable["COD2"] = gameColorTable["COD"];
	gameColorTable["CODUO"] = gameColorTable["COD"];
	gameColorTable["COD4MW"] = gameColorTable["COD"];
	gameColorTable["COD5"] = gameColorTable["COD"];

	function colorizeNameWithColorTableCarat(name, colorTable)
	{
		return colorizeNameWithColorTable(name,colorTable,'^');
	}

	function colorizeNameWithColorTableDollar(name, colorTable)
	{
		return colorizeNameWithColorTable(name,colorTable,'$');
	}

	function colorizeNameWithColorTable(name, colorTable, colorChar)
	{
		if (name == null || name == "")
			return "";
	
		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			if (name.charAt(i) == colorChar && (i+1) < name.length)
			{
				if (name.charAt(i+1).match(/^\d$/))
				{
					var index = parseInt(name.charAt(i+1));
					i++;
					if (colorTable[index] != null)
					{
						ret += "</span><span ";
						ret += "style=\"color: " + colorTable[index] + "\"";
						ret += ">";
					}
					continue;

				}
			}
			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	function colorizeNameUT(name)
	{
		if (name == null || name == "")
			return "";

		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			if (name.charCodeAt(i) == 27 && (i+3) < name.length)
			{
				i++;
				ret += "</span><span ";
				ret += "style=\"color: rgb(";
				ret += name.charCodeAt(i) + ",";
				i++;
				ret += name.charCodeAt(i) + ",";
				i++;
				ret += name.charCodeAt(i) + ")\"";
				ret += ">";
				continue;
			}

			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	function colorizeNameSOF(name)
	{
		if (name == null || name == "")
			return "";

		var colors = [ "black", 
	"#FFFFFF","#FF0000","#00FF00","#FFFF00","#0000FF","#FF00FF",
	"#00FFFF","#000000","#7F7F7F","#702D07","#7F0000","#007F00",
	"#007F7F","#00007F","#564D28","#4C5E36","#370B65","#005572",
	"#54647E","#1E2A63","#66097B","#705E61","#980053","#960018",
	"#702D07","#54492A","#61A997","#CB8F39","#CF8316","#FF8020" ];


		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			var code = name.charCodeAt(i);

			if (code < colors.length)
			{
				ret += "</span><span ";
				ret += "style=\"color: " + colors[code];
				ret += ";\">";
				continue;
			}
			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	function colorizeNameQuakeWorld(name)
	{
		if (name == null || name == "")
			return "";

		var charlookup =
			"·    ·       >··" + 
			"[]0123456789·   " +
			" !\"#$%&'()*+,-./" +
			"0123456789:;(=)?" +
			"@ABCDEFGHIJKLMNO" +
			"PQRSTUVWXYZ[\\]^_" +
			"'abcdefghijklmno" +
			"pqrstuvwxyz{|}~" +
			"     ·       >··" +
			"[]0123456789·   " +
			" !\"#$%&'()*+,-./" +
			"0123456789:;(=)?" +
			"@ABCDEFGHIJKLMNO" +
			"PQRSTUVWXYZ[\\]^_" +
			"'abcdefghijklmno" +
			"pqrstuvwxyz{|}~";

		var colorlookup =
			"             r  " +
			"oogggggggggg    " +
			"                " +
			"                " +
			"                " +
			"                " +
			"                " +
			"                " +
			"     g       rgg" +
			"ooggggggggggg   " +
			"rrrrrrrrrrrrrrrr" +
			"rrrrrrrrrrrrrrrr" +
			"rrrrrrrrrrrrrrrr" +
			"rrrrrrrrrrrrrrrr" +
			"rrrrrrrrrrrrrrrr" +
			"rrrrrrrrrrrrrrrr";

		var i;
		var color = " ";
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			var code = name.charCodeAt(i);

			if (code >= charlookup.length)
				continue;

			var new_char = charlookup.charAt(code);
			var new_color = colorlookup.charAt(code);

			if (new_color != color)
			{
				ret += "</span><span ";

				color = new_color;
				if (color == " ")
					;
				if (color == "g")
					ret += "style=\"color: rgb(140,112,56);\"";
				if (color == "o")
					ret += "style=\"color: rgb(216,144,72);\"";
				if (color == "r")
					ret += "style=\"color: rgb(200,124,72);\"";
				ret += ">";

			}
			ret += new_char;
		}
		ret += "</span>";
		return ret;
			
	}
	
	function colorizeNameQuake3(name)
	{
		// everything after a carat is a color code, but different
		// per game. We just handle the numbers for now

		if (name == null || name == "")
			return "";
	
		var colorTable = gameColorTable["Q3A"];

		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			if (name.charAt(i) == '^' && (i+1) < name.length)
			{
				if (name.charAt(i+1).match(/^\d$/))
				{
					var index = parseInt(name.charAt(i+1));
					i++;
					if (colorTable[index] != null)
					{
						ret += "</span><span ";
						ret += "style=\"color: " + colorTable[index] + "\"";
						ret += ">";
					}
					continue;
				}
				else
				{
					// skip unknown color code
					i++;
					continue;
				}
			}
			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	function Quake4CodeToColor(char_code)
	{
		if (char_code <= 48)
			char_code = 0;
		else
		{
			if (char_code > 57)
				char_code = 58;
			char_code = char_code - 48;
		}
		return 255*char_code/10;
	}

	function colorizeNameQuake4(name)
	{
		// quake 4 supports the old ^[0-9] colors, plus a new wrinkle
		if (name == null || name == "")
			return "";

		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			if (name.charAt(i) == '^' && (i+1) < name.length)
			{
				if (name.charAt(i+1).match(/^\d$/))
				{
					var index = parseInt(name.charAt(i+1));
					i++;
					if (gameColorTable["Q4"][index] != null)
					{
						ret += "</span><span ";
						ret += "style=\"color: " + gameColorTable["Q4"][index] + "\"";
						ret += ">";
					}
					continue;
				}
				else if ((name.charAt(i+1) == 'c' || name.charAt(i+1) == 'C') && i+4 < name.length)
				{
					var r = Quake4CodeToColor(name.charCodeAt(i+2));
					var g = Quake4CodeToColor(name.charCodeAt(i+3));
					var b = Quake4CodeToColor(name.charCodeAt(i+4));

					ret += "</span><span ";
					ret += "style=\"color: rgb(" + r + "," + g + "," + b + ")\"";
					ret += ">";
					i += 4;
					continue;
				}
				else if ((name.charAt(i+1) == 'i' || name.charAt(i+1) == 'I') && i+4 < name.length)
				{
					if (name.charAt(i+2) == 'w' || name.charAt(i+2) == 'W')
					{
						if (name.charAt(i+4) >= '0' && name.charAt(i+4) <= '9')
						{
							ret += "<img src=\"%media_template_folder%infoview\\quake4\\iw0" + name.charAt(i+4) + ".gif\" alt=\"\">";
						}
					}
					else if (((name.charAt(i+2) == 'd' || name.charAt(i+2) == 'D')) && ((name.charAt(i+3) == 'm' || name.charAt(i+3) == 'M')))
					{
						if (name.charAt(i+4) >= '0' && name.charAt(i+4) <= '1')
						{
							ret += "<img src=\"%media_template_folder%infoview\\quake4\\idm" + name.charAt(i+4) + ".gif\" alt=\"\">";
						}
					}
					i += 4;
					continue;
				}
				else if ((name.charAt(i+1) == 'n' || name.charAt(i+1) == 'N') && i+2 < name.length)
				{
					// not sure what this does
					i += 2;
					continue;
				}
				else if (name.charAt(i+1) == '-')
				{
					// slightly dimmer, should deal with this
					i++;
					continue;
				}
				else if (name.charAt(i+1) == '+')
				{
					// slightly brighter, should deal with this
					i++;
					continue;
				}
				else if (name.charAt(i+1) == 'r' || name.charAt(i+1) == 'R')
				{
					// reset?
					ret += "</span><span ";
					ret += "style=\"color: " + gameColorTable["Q4"][0] + "\"";
					ret += ">";
					i++;
					continue;
				}
			}

			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	function ETQWColorCodeToColor(code)
	{
		var data = { 
			"1": "#FD0000",
			"2": "#02FA06",
			"3": "#FAFF03",
			"4": "#0001FC",
			"5": "#0AFEFF",
			"6": "#FF00F9",
			"7": "#FFFFFF",
			"8": "#6C6D6F",
			"9": "#000000",
			"0": "#FFFFFF",
			"a": "#DD8517",
			"b": "#008180",
			"c": "#6D0469",
			"d": "#D56A00",
			"e": "#6C00DF",
			"f": "#3295D0",
			"g": "#A8D5B4",
			"h": "#006830",
			"i": "#FD0000",
			"j": "#B01D23",
			"k": "#973B00",
			"l": "#C59A31",
			"m": "#96952B",
			"n": "#FEFFBE",
			"o": "#DCD46F",
			"p": "#000000",
			"q": "#FD0000",
			"r": "#02FA06",
			"s": "#FAFF03",
			"t": "#0001FC",
			"u": "#0AFEFF",
			"v": "#FF00F9",
			"w": "#FFFFFF",
			"x": "#6C6D6F",
			"y": "#000000",
			"z": "#BCC1C5"
		};
		return data[code];
	}
	function colorizeNameETQW(name)
	{
		// quake 4 supports the old ^[0-9] colors, plus a new wrinkle
		if (name == null || name == "")
			return "";

		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			if (name.charAt(i) == '^' && (i+1) < name.length)
			{
				var ch = name.charAt(i+1);
				ret += "</span><span ";
				ret += "style=\"color: " + ETQWColorCodeToColor(ch) + "\"";
				ret += ">";				
				i++;
				continue;
			}
			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}
	function colorizeNameSWAT4(name)
	{
		if (name == null || name == "")
			return "";

		var in_underline = 0;
		var in_bold = 0;
		var color = "";
		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			// SWAT 4 has some crazy codes--
			// [u] starts underline
			// [b] starts bold
			// [c=xxxxxx] sets rgb hex color xxxxxx
			var respan = 0;
			if ((i + 2) < name.length && (name.substring(i,i+3) == "[u]" || name.substring(i,i+3) == "[U]"))
			{
				in_underline = 1;
				i += 2;
				respan = 1;
			}
			if ((i + 2) < name.length && (name.substring(i,i+3) == "[b]" || name.substring(i,i+3) == "[B]"))
			{
				in_bold = 1;
				i += 2;
				respan = 1;
			}
			if ((i + 9) < name.length && name.charAt(i) == '[' &&
			    (name.charAt(i+1) == 'c' || name.charAt(i+1) == 'C') && name.charAt(i+9) == ']')
			{
				color = name.substring(i+3,i+9);
				i += 9;
				respan = 1;
			}

			if (respan)
			{
				ret += "</span><span ";
				ret += "style=\"";
				if (in_underline)
					ret += "text-decoration:underline; ";
				if (in_bold)
					ret += "font-weight:bold; ";
				ret += "color: #";
				ret += color;
				ret += ";\">";
				continue;
			}

			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	function colorizeNameWolfET(name)
	{
		if (name == null || name == "")
			return "";
	
		var colorTable = gameColorTable["WOLFET"];

		var i;
		var ret = "<span>";
		for (i = 0; i < name.length; i++)
		{
			if (name.charAt(i) == '^' && (i+1) < name.length)
			{
				var nextchar = name.charAt(i+1);
				if (nextchar != '^')
				{
					// treat "^&lt;" like "^<", etc.
					var ents = { '<': '&lt;', '>': '&gt;', '"': '&quot;', '&': '&amp;' };
					for (var e in ents)
					{
						if (name.substr(i+1, ents[e].length) == ents[e])
						{
							nextchar = e;
							i += ents[e].length - 1;
							break;
						}
					}
					i++;
					
					if (nextchar == '*')
						ret += "</span><span>";		// reset
					else
					{
						var index = (nextchar.charCodeAt(0) - '0'.charCodeAt(0) + 2*colorTable.length) % colorTable.length;
						ret += "</span><span ";
						ret += "style=\"color: " + colorTable[index] + "\"";
						ret += ">";
					}
					continue;
				}

				// show this caret
			}
			ret += name.charAt(i);
		}
		ret += "</span>";
		return ret;
	}

	// name is a server name or player name
	function colorizeName(name, serverType)
	{
		if (serverType == "UT2K3" || serverType == "AA")
		{
			return colorizeNameUT(name);
		}
		if (serverType == "SWAT4")
		{
			return colorizeNameSWAT4(name);
		}
		if (serverType == "QW")
		{
			return colorizeNameQuakeWorld(name);
		}
		if (serverType == "Q3A")
		{
			return colorizeNameQuake3(name);
		}
		if (serverType == "Q4")
		{
			return colorizeNameQuake4(name);
		}
		if (serverType == "ETQW")
		{
			return colorizeNameETQW(name);
		}
		if (serverType == "SOF")
		{
			return colorizeNameSOF(name);
		}
		if (serverType == "WOLFET")
		{
			return colorizeNameWolfET(name);
		}

		var colorTable = gameColorTable[serverType];
		if (colorTable == null)
		{
			return name;
		}
		if (serverType == "FARCRY")
			return colorizeNameWithColorTableDollar(name, colorTable);
		return colorizeNameWithColorTableCarat(name, colorTable);
	}
