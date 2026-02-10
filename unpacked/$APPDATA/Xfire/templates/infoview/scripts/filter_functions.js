		function splitEscaped(str,delim,esc_char)
		{
			var ret = new Array();
			var s = 0;
			if (delim == null)
			{
				delim = ';';
			}
			if (esc_char == null)
			{
				esc_char = false;
			}
			for(var e = 0; e < str.length; e++) {
				if (str.charAt(e) == '\\' && (str.charAt(e+1) == delim ||
							      (esc_char && str.charAt(e+1) == '\\'))) {
					str = str.substring(0,e) + str.substring(e+1);
					continue;
				}
				if (str.charAt(e) == delim) {
					ret.push(str.substring(s,e));
					s = e+1;
				}
			}
			if (str.substring(s) != '') {
				ret.push(str.substring(s));
			}
			return ret;
		}

		function escapeString(str)
		{
			var ret = "";
			for(var e = 0; e < str.length; e++) {
				if (str.charAt(e) == '\\' || str.charAt(e) == ';') {
					ret += '\\';
				}
				ret += str.charAt(e);
			}
			return ret;
		}

		function unescapeString(str)
		{
			var ret = "";
			var s = 0;
			for (var e = 0; e < str.length; e++) {
				if (str.charAt(e) == '\\')
				{
					ret += str.substring(s,e);
					e++;
					s = e;
				}
			}
			if (str.substring(s) != '')
			{
				ret += str.substring(s);
			}
			return ret;
		}

		function parseFilter(filter)
		{
			for (var e = 0; e < filter.length; e++) {
				if (
				    (filter.charAt(e) == '=' && filter.charAt(e+1) == '=') ||
				    (filter.charAt(e) == '!' && filter.charAt(e+1) == '=') ||
				    (filter.charAt(e) == '>' && filter.charAt(e+1) == '=') ||
				    (filter.charAt(e) == '<' && filter.charAt(e+1) == '=') ||
				    (filter.charAt(e) == '~' && filter.charAt(e+1) == '~')
				   )
				{
					return new Array(unescapeString(filter.substring(0, e)), filter.substring(e, e+2), unescapeString(filter.substring(e+2)));
				}
			}
			return null;
		}
		